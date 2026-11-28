// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
//
// Boundary-Contract:
// - Owns:
//   - Android application entry, scoped launch context and process output.
// - Must-Not:
//   - Interpret title options, simulation rules or native media.
// - Allows:
//   - Inputs: Explicit installed artifact and Java launch arguments.
//   - Outputs: Owned process observations and resultful launch completion.
//   - Side effects: One Rust-owned worker and bounded Java diagnostics.
// - Split-When:
//   - Process output and entry need separate native owners.
// - Merge-When:
//   - A shared adapter owns identical process observations.
// - Summary:
//   - Enter one registered application through the Android Activity.
// - Description:
//   - The installed shared object, not app_process, identifies the title.
// - Usage:
//   - Launch through the platform-owned Java callback.
// - Defaults:
//   - Reject missing context, conflicting launches and invalid artifacts.

//! Android application entry and process adapter.

use std::ffi::{OsStr, OsString};
use std::fmt::Arguments;
use std::io::{Error, ErrorKind, Result as IoResult};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use jni::errors::{Error as JavaError, JniError, ThrowRuntimeExAndDefault};
use jni::objects::{JClass, JObjectArray, JString};
use jni::strings::JNIStr;
use jni::sys::jint;
use jni::{Env, EnvUnowned, JValue, JavaVM, jni_mangle, jni_sig, jni_str};

use crate::ir::process::ProcessPort;
use crate::ir::process::application::launch_registered;
use crate::ir::storage::{
    PathKind, canonical_path, path_is_absolute, path_kind, paths_overlap,
};
use crate::software::linux::storage::application_data_directory;

// The Java launcher supplies one artifact and an explicitly bounded list of
// arguments. Reject excess inputs rather than narrowing them.
const MAX_ARGUMENTS: usize = 1024;
const MAX_ARGUMENT_BYTES: usize = 131_072;
// Android log entries have a finite payload including their tag and headers.
// Bound UTF-8 chunks conservatively before conversion to modified UTF-8.
const LOG_CHUNK_BYTES: usize = 2000;

static CONTEXT: Mutex<Option<Context>> = Mutex::new(None);

type JavaResult<T> = Result<T, Box<JavaError>>;
struct Context {
    arguments: Vec<OsString>,
    artifact: OsString,
    data: OsString,
    vm: JavaVM,
}

struct LaunchScope;

impl Drop for LaunchScope {
    fn drop(&mut self) {
        // A panic must not leave an admitted context available to a retry.
        if let Ok(mut context) = CONTEXT.lock() {
            drop(context.take());
        }
    }
}

impl LaunchScope {
    fn enter(context: Context) -> IoResult<Self> {
        let mut active = locked_context()?;
        if active.is_some() {
            return Err(ErrorKind::AlreadyExists.into());
        }
        *active = Some(context);
        drop(active);
        Ok(Self)
    }
}

fn locked_context() -> IoResult<MutexGuard<'static, Option<Context>>> {
    CONTEXT
        .lock()
        .map_err(|_poisoned| Error::other("application context lock poisoned"))
}

fn observe<Value, Read>(read: Read) -> IoResult<Value>
where
    Read: FnOnce(&Context) -> Value,
{
    locked_context()?
        .as_ref()
        .map(read)
        .ok_or_else(|| ErrorKind::Unsupported.into())
}

pub(in crate::software) fn application_data_path() -> IoResult<OsString> {
    observe(|context| context.data.clone())
}

pub(in crate::software) fn executable_path() -> IoResult<OsString> {
    observe(|context| context.artifact.clone())
}

impl ProcessPort for () {
    fn arguments() -> IoResult<Vec<OsString>> {
        observe(|context| context.arguments.clone())
    }

    fn wait_for_retry(delay: Duration) -> IoResult<()> {
        thread::sleep(delay);
        Ok(())
    }

    fn write_diagnostic(message: Arguments<'_>) -> IoResult<()> {
        write_log(jni_str!("e"), message)
    }

    fn write_status(message: Arguments<'_>) -> IoResult<()> {
        write_log(jni_str!("i"), message)
    }
}

fn write_log(level: &JNIStr, message: Arguments<'_>) -> IoResult<()> {
    let vm = observe(|context| context.vm.clone())?;
    let text = message.to_string();
    let mut remaining = text.as_str();
    loop {
        let end = remaining.floor_char_boundary(LOG_CHUNK_BYTES);
        let (part, next) = remaining.split_at(end);
        log_record(&vm, level, part)?;
        if next.is_empty() {
            return Ok(());
        }
        remaining = next;
    }
}

fn log_record(vm: &JavaVM, level: &JNIStr, message: &str) -> IoResult<()> {
    let written = vm
        .attach_current_thread_for_scope(|env| -> JavaResult<i32> {
            let tag = env.new_string("Games")?;
            let text = env.new_string(message)?;
            Ok(env
                .call_static_method(
                    jni_str!("android/util/Log"),
                    level,
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)I"),
                    &[JValue::Object(&tag), JValue::Object(&text)],
                )?
                .i()?)
        })
        .map_err(Error::other)?;
    if written <= 0i32 {
        return Err(Error::other("Android log rejected message"));
    }
    Ok(())
}

fn admitted_data(artifact: &OsStr, parent: &OsStr) -> IoResult<OsString> {
    if !path_is_absolute(parent)? || path_kind(parent)? != PathKind::Directory {
        return Err(ErrorKind::InvalidInput.into());
    }
    let canonical = canonical_path(parent)?;
    let library = Path::new(artifact)
        .parent()
        .ok_or(ErrorKind::InvalidInput)?;
    if paths_overlap(&canonical, library.as_os_str())? {
        return Err(ErrorKind::InvalidInput.into());
    }
    application_data_directory(Path::new(&canonical))
        .map(PathBuf::into_os_string)
}

fn installed_artifact(path: &OsStr) -> IoResult<OsString> {
    if !path_is_absolute(path)? || path_kind(path)? != PathKind::RegularFile {
        return Err(ErrorKind::InvalidInput.into());
    }
    let resolved = canonical_path(path)?;
    if path_kind(&resolved)? != PathKind::RegularFile {
        return Err(ErrorKind::InvalidData.into());
    }
    Ok(resolved)
}

const fn invalid_arguments() -> JavaError {
    JavaError::JniCall(JniError::InvalidArguments)
}

fn launch_arguments(
    env: &mut Env<'_>,
    arguments: &JObjectArray<'_, JString<'_>>,
) -> JavaResult<Vec<OsString>> {
    let count = arguments.len(env)?;
    if count > MAX_ARGUMENTS {
        return Err(invalid_arguments().into());
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_error| invalid_arguments())?;
    let mut bytes = 0usize;
    for index in 0..count {
        let string = arguments.get_element(env, index)?;
        let argument = string.try_to_string(env)?;
        env.delete_local_ref(string);
        bytes = bytes
            .checked_add(argument.len())
            .ok_or_else(invalid_arguments)?;
        if bytes > MAX_ARGUMENT_BYTES || argument.contains('\0') {
            return Err(invalid_arguments().into());
        }
        result.push(OsString::from(argument));
    }
    Ok(result)
}

fn run_application(context: Context) -> IoResult<()> {
    let arguments = context.arguments.clone();
    let scope = LaunchScope::enter(context)?;
    let result = thread::Builder::new()
        .name(String::from("games-application"))
        .spawn(move || launch_registered(arguments))?
        .join()
        .map_err(|_panic| Error::other("application worker panicked"))?
        .map_err(|error| Error::other(format!("{error:?}")));
    if let Err(error) = &result {
        write_log(jni_str!("e"), format_args!("{error}"))?;
    }
    drop(scope);
    result
}

/// Android Activity callback. The Rust worker owns its SDL attachment.
#[expect(
    unreachable_pub,
    reason = "JNI export requires public visibility but is not a Rust API"
)]
#[jni_mangle("org.games.runtime.GamesActivity")]
pub fn run_native<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    artifact: JString<'local>,
    data_parent: JString<'local>,
    arguments: JObjectArray<'local, JString<'local>>,
) -> jint {
    unowned_env
        .with_env(|env| -> JavaResult<_> {
            let supplied = artifact.try_to_string(env)?;
            let vm = env.get_java_vm()?;
            let installed = installed_artifact(OsStr::new(&supplied))
                .map_err(|_error| invalid_arguments())?;
            let data = admitted_data(
                &installed,
                OsStr::new(&data_parent.try_to_string(env)?),
            )
            .map_err(|_error| invalid_arguments())?;
            let context = Context {
                arguments: launch_arguments(env, &arguments)?,
                artifact: installed,
                data,
                vm,
            };
            Ok(if run_application(context).is_ok() {
                0i32
            } else {
                -1i32
            })
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}
