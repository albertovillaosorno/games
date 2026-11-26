// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
// Boundary-Contract:
// - Owns:
//   - Explicit browser directory selection and bounded file transfer.
// - Must-Not:
//   - Resolve host paths, fetch remote assets or interpret title catalogues.
// - Allows:
//   - Inputs: User-selected browser files and portable admission plan.
//   - Outputs: Admitted memory assets and selected sidecar snapshots.
//   - Side effects: Scoped picker and one asynchronous file read at a time.
// - Split-When:
//   - A browser storage mechanism needs independent selection semantics.
// - Merge-When:
//   - A shared host picker owns the same file selection and transfer.
// - Summary:
//   - Read only the files granted by the browser directory chooser.
// - Description:
//   - Preflight all relative names and sizes before allocating file data.
// - Usage:
//   - Present the picker through the portable directory capability.
// - Defaults:
//   - No network requests or persistent copies; cancellation fails closed.

use std::io::ErrorKind;

use games_assets::AssetError;
use games_assets::archive::validate_path;
use js_sys::futures::spawn_local;
use js_sys::{
    ArrayBuffer, Function, JsString, Number, Promise, Reflect, Uint8Array,
};
use wasm_bindgen::{JsCast as _, JsValue};
use web_sys::{Element, Event, File, FileList, HtmlInputElement};

use crate::ir::security::file::allocation_length;
use crate::ir::storage::bundle::BundleError;
use crate::ir::storage::bundle::directory::{
    DirectoryComplete, DirectoryPicker, DirectoryPlan, DirectoryRead,
    DirectorySelectionError, DirectorySelector, PlannedFile, SelectedDirectory,
};

type Transfer = (File, PlannedFile);
type Preflight = (DirectoryRead, Vec<Transfer>);

const PATH_UNITS: u32 = 4352;

struct Picker {
    callback: Function,
    cancel: bool,
    change: bool,
    container: Element,
    input: HtmlInputElement,
}

impl Picker {
    fn new() -> Result<(Self, Promise), BundleError> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or(BundleError::Io(ErrorKind::Unsupported))?;
        let body = document
            .body()
            .ok_or(BundleError::Io(ErrorKind::Unsupported))?;
        let container =
            document.create_element("label").map_err(host_failure)?;
        container
            .set_text_content(Some("Select the canonical assets directory: "));
        let input = document
            .create_element("input")
            .map_err(host_failure)?
            .dyn_into::<HtmlInputElement>()
            .map_err(|_element| BundleError::UnsafeStorage)?;
        input.set_attribute("type", "file").map_err(host_failure)?;
        if !Reflect::has(input.as_ref(), &JsValue::from_str("webkitdirectory"))
            .map_err(host_failure)?
        {
            return Err(BundleError::Io(ErrorKind::Unsupported));
        }
        input
            .set_attribute("webkitdirectory", "")
            .map_err(host_failure)?;
        let _child = container.append_child(&input).map_err(host_failure)?;
        let mut callback = None;
        let promise = Promise::new(&mut |resolve, _reject| {
            callback = Some(resolve);
        });
        let mut picker = Self {
            callback: callback
                .ok_or(BundleError::Io(ErrorKind::Unsupported))?,
            cancel: false,
            change: false,
            container,
            input,
        };
        picker
            .input
            .add_event_listener_with_callback("change", &picker.callback)
            .map_err(host_failure)?;
        picker.change = true;
        picker
            .input
            .add_event_listener_with_callback("cancel", &picker.callback)
            .map_err(host_failure)?;
        picker.cancel = true;
        let _child =
            body.append_child(&picker.container).map_err(host_failure)?;
        Ok((picker, promise))
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        for (registered, event) in
            [(self.change, "change"), (self.cancel, "cancel")]
        {
            if registered {
                let _removed = self
                    .input
                    .remove_event_listener_with_callback(event, &self.callback);
            }
        }
        self.container.remove();
    }
}

impl DirectoryPicker for () {
    fn select<Error>(
        plan: DirectoryPlan,
        choose: DirectorySelector<Error>,
        complete: DirectoryComplete<Error>,
    ) -> Result<(), BundleError>
    where
        Error: 'static,
    {
        let (picker, event) = Picker::new()?;
        spawn_local(async move {
            let result = drive(picker, event, plan, choose).await;
            complete(result);
        });
        Ok(())
    }
}

async fn drive<Error>(
    picker: Picker,
    event_promise: Promise,
    plan: DirectoryPlan,
    choose: DirectorySelector<Error>,
) -> Result<SelectedDirectory, DirectorySelectionError<Error>> {
    let event = event_promise
        .await
        .map_err(host_failure)?
        .dyn_into::<Event>()
        .map_err(host_failure)?;
    if !event.is_trusted() {
        return Err(BundleError::UnsafeStorage.into());
    }
    match event.type_().as_str() {
        "cancel" => return Err(BundleError::Io(ErrorKind::Interrupted).into()),
        "change" => {},
        _ => return Err(BundleError::UnsafeStorage.into()),
    }
    let files = property(picker.input.as_ref(), "files")?
        .dyn_into::<FileList>()
        .map_err(host_failure)?;
    let (mut reader, transfers) = preflight(plan, &files)?;
    drop(picker);
    read_pending(&mut reader, &transfers).await?;
    let requests = choose(reader.sidecars()?)
        .map_err(DirectorySelectionError::Selection)?;
    reader.request_manifests(&requests)?;
    read_pending(&mut reader, &transfers).await?;
    reader.request_objects()?;
    read_pending(&mut reader, &transfers).await?;
    Ok(reader.finish()?)
}

async fn read_pending(
    reader: &mut DirectoryRead,
    transfers: &[Transfer],
) -> Result<(), BundleError> {
    for (file, planned) in transfers {
        if reader.wants(planned) {
            let bytes = read_file(file, planned.size()).await?;
            reader.receive(planned, bytes)?;
        }
    }
    Ok(())
}

fn preflight(
    mut plan: DirectoryPlan,
    files: &FileList,
) -> Result<Preflight, BundleError> {
    let count = u32::try_from(integer(property(files.as_ref(), "length")?)?)
        .map_err(|_error| AssetError::TooLarge)?;
    let capacity =
        usize::try_from(count).map_err(|_error| AssetError::TooLarge)?;
    if capacity > plan.file_limit().get() {
        return Err(AssetError::TooLarge.into());
    }
    let mut transfers = Vec::new();
    transfers
        .try_reserve_exact(capacity)
        .map_err(|_error| AssetError::TooLarge)?;
    let mut root = None;
    for index in 0..count {
        let file = Reflect::get(files.as_ref(), &JsValue::from(index))
            .map_err(host_failure)?
            .dyn_into::<File>()
            .map_err(host_failure)?;
        let path = file_path(&file)?;
        let relative = relative_path(&path, &mut root)?;
        let size = integer(property(file.as_ref(), "size")?)?;
        if let Some(planned) = plan.observe(relative, size)? {
            transfers.push((file, planned));
        }
    }
    Ok((plan.begin()?, transfers))
}

fn file_path(file: &File) -> Result<String, BundleError> {
    let raw = property(file.as_ref(), "webkitRelativePath")?;
    let text = raw
        .dyn_ref::<JsString>()
        .ok_or(BundleError::UnsafeStorage)?;
    if text.length() > PATH_UNITS {
        return Err(AssetError::TooLarge.into());
    }
    raw.as_string().ok_or(BundleError::UnsafeStorage)
}

fn relative_path<'path>(
    path: &'path str,
    root: &mut Option<String>,
) -> Result<&'path str, BundleError> {
    let (name, relative) =
        path.split_once('/').ok_or(BundleError::UnsafeStorage)?;
    validate_path(name).map_err(|_error| BundleError::UnsafeStorage)?;
    if root.as_ref().is_some_and(|expected| expected != name) {
        return Err(BundleError::UnsafeStorage);
    }
    if root.is_none() {
        *root = Some(name.to_owned());
    }
    Ok(relative)
}

#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "nonnegative safe JS integers are checked before conversion"
)]
fn integer(value: JsValue) -> Result<u64, BundleError> {
    if !Number::is_safe_integer(&value) {
        return Err(BundleError::UnsafeStorage);
    }
    let observed = value.as_f64().ok_or(BundleError::UnsafeStorage)?;
    if observed < 0.0 {
        return Err(BundleError::UnsafeStorage);
    }
    Ok(observed as u64)
}

fn property(object: &JsValue, name: &str) -> Result<JsValue, BundleError> {
    Reflect::get(object, &JsValue::from_str(name)).map_err(host_failure)
}

async fn read_file(file: &File, size: u64) -> Result<Vec<u8>, BundleError> {
    let length = allocation_length(size)?;
    let method = property(file.as_ref(), "arrayBuffer")?
        .dyn_into::<Function>()
        .map_err(host_failure)?;
    let promise = method
        .call0(file.as_ref())
        .map_err(host_failure)?
        .dyn_into::<Promise>()
        .map_err(host_failure)?;
    let buffer = promise
        .await
        .map_err(host_failure)?
        .dyn_into::<ArrayBuffer>()
        .map_err(host_failure)?;
    if u64::from(buffer.byte_length()) != size {
        return Err(AssetError::PayloadMismatch.into());
    }
    let mut bytes = Vec::new();
    if length > 0 {
        let view = Uint8Array::new(buffer.as_ref());
        if u64::from(view.length()) != size {
            return Err(AssetError::PayloadMismatch.into());
        }
        bytes
            .try_reserve_exact(length)
            .map_err(|_error| AssetError::TooLarge)?;
        bytes.resize(length, 0);
        view.copy_to(&mut bytes);
    }
    Ok(bytes)
}

fn host_failure(_error: JsValue) -> BundleError {
    BundleError::Io(ErrorKind::Other)
}
