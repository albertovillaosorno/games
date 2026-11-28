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
//   - Exact SDL2 Activity adaptation and authored Android manifest generation.
// - Must-Not:
//   - Modify the upstream registry, title code or original media.
// - Allows:
//   - Inputs: Locked upstream source and an admitted title identifier.
//   - Outputs: Private Java source and launcher manifest.
//   - Side effects: None.
// - Split-When:
//   - Another Android entry needs independent source adaptation.
// - Merge-When:
//   - Upstream SDL provides the required safe application entry.
// - Summary:
//   - Adapt the pinned Java runner to the platform-owned JNI entry.
// - Description:
//   - Preserve Activity ownership and reject unknown source identities.
// - Usage:
//   - Generate inputs for the authored Android package.
// - Defaults:
//   - Debug-only prototype; no external storage or backup permission.

use games_assets::ContentHash;

const SDL_ACTIVITY_HASH: &str =
    "12224e9eb8e659dde6831ab260fb3eeefed74fca72baeef1182402d74cb37fd5";
const MAIN_DECLARATION: &str = concat!(
    "    /**\n",
    "     * This method returns the name of the application entry point\n",
    "     * It can be overridden by derived classes.\n",
    "     */\n",
    "    protected String getMainFunction() {",
);
const MAIN_HOOK: &str = concat!(
    "    // Games modification: delegate SDL's runner to the safe JNI entry.\n",
    "    protected void runApplication(String library, String function,\n",
    "                                  String[] arguments) {\n",
    "        SDLActivity.nativeRunMain(library, function, arguments);\n",
    "    }\n\n",
);
const ORIGINAL_CALL: &str =
    "        SDLActivity.nativeRunMain(library, function, arguments);";
const DELEGATE_CALL: &str =
    "        owner.runApplication(library, function, arguments);";
const ORIGINAL_RUNNER: &str = concat!(
    "        String library = SDLActivity.mSingleton.getMainSharedObject();\n",
    "        String function = SDLActivity.mSingleton.getMainFunction();\n",
    "        String[] arguments = SDLActivity.mSingleton.getArguments();\n",
);
const OWNED_RUNNER: &str = concat!(
    "        final SDLActivity owner = SDLActivity.mSingleton;\n",
    "        final Thread worker = Thread.currentThread();\n",
    "        String library = owner.getMainSharedObject();\n",
    "        String function = owner.getMainFunction();\n",
    "        String[] arguments = owner.getArguments();\n",
);
const ORIGINAL_FINISH: &str = concat!(
    "        if (SDLActivity.mSingleton != null",
    " && !SDLActivity.mSingleton.isFinishing()) {\n",
    "            // Let's finish the Activity\n",
    "            SDLActivity.mSDLThread = null;\n",
    "            SDLActivity.mSingleton.finish();\n",
    "        }  // else: Activity is already being destroyed\n",
);
const GUARDED_FINISH: &str = concat!(
    "        // Games modification: a retiring runner must not finish",
    " a replacement.\n",
    "        owner.runOnUiThread(() -> {\n",
    "            if (SDLActivity.mSingleton == owner\n",
    "                    && SDLActivity.mSDLThread == worker\n",
    "                    && !owner.isFinishing()\n",
    "                    && !owner.isDestroyed()\n",
    "                    && !owner.isChangingConfigurations()) {\n",
    "                SDLActivity.mSDLThread = null;\n",
    "                owner.finish();\n",
    "            }\n",
    "        });\n",
);

pub(super) fn adapt_activity(source: &str) -> Option<String> {
    if ContentHash::calculate(source.as_bytes()).to_string()
        != SDL_ACTIVITY_HASH
    {
        return None;
    }
    delegate(source)
}

fn delegate(source: &str) -> Option<String> {
    if [
        MAIN_DECLARATION,
        ORIGINAL_RUNNER,
        ORIGINAL_CALL,
        ORIGINAL_FINISH,
    ]
    .into_iter()
    .any(|fragment| source.matches(fragment).count() != 1)
        || source.contains("runApplication(")
    {
        return None;
    }
    // The UI thread owns Activity destruction and the SDL worker reference.
    // A worker ending during recreation cannot finish the replacement.
    let delegated = source
        .replace(ORIGINAL_RUNNER, OWNED_RUNNER)
        .replace(ORIGINAL_CALL, DELEGATE_CALL)
        .replace(ORIGINAL_FINISH, GUARDED_FINISH);
    Some(
        delegated.replace(
            MAIN_DECLARATION,
            &format!("{MAIN_HOOK}{MAIN_DECLARATION}"),
        ),
    )
}

pub(super) fn manifest(title: &str) -> Option<String> {
    if title.is_empty()
        || !title.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
        })
    {
        return None;
    }
    let application = title.replace('-', "_");
    Some(format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
            "<manifest xmlns:android=\"http://schemas.android.com",
            "/apk/res/android\"\n",
            "    package=\"org.games.title_{application}\"\n",
            "    android:versionCode=\"1\" android:versionName=\"0.1.0\">\n",
            "  <uses-sdk android:minSdkVersion=\"24\" ",
            "android:targetSdkVersion=\"35\" />\n",
            "  <uses-feature android:glEsVersion=\"0x00020000\" />\n",
            "  <uses-feature android:name=\"android.hardware.touchscreen\" ",
            "android:required=\"false\" />\n",
            "  <application android:label=\"Games\" ",
            "android:allowBackup=\"false\"\n",
            "      android:theme=\"@android:style/",
            "Theme.Material.NoActionBar.Fullscreen\"\n",
            "      android:debuggable=\"true\" ",
            "android:hardwareAccelerated=\"true\" ",
            "android:extractNativeLibs=\"true\">\n",
            "    <activity android:name=\"org.games.runtime.GamesActivity\"\n",
            "        android:exported=\"true\" ",
            "android:launchMode=\"singleInstance\"\n",
            "        android:alwaysRetainTaskState=\"true\"\n",
            "        android:configChanges=\"orientation|screenSize",
            "|screenLayout",
            "|keyboard|keyboardHidden|navigation|uiMode|locale",
            "|layoutDirection|smallestScreenSize\">\n",
            "      <intent-filter>\n",
            "        <action android:name=\"android.intent.action.MAIN\" />\n",
            "        <category android:name=\"android.intent.category.",
            "LAUNCHER\" />\n",
            "      </intent-filter>\n",
            "    </activity>\n",
            "  </application>\n",
            "</manifest>\n",
        ),
        application = application,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_delegation_preserves_default_upstream_entry() -> Result<(), String>
    {
        let source = [
            MAIN_DECLARATION,
            "\n}\n",
            ORIGINAL_RUNNER,
            ORIGINAL_CALL,
            "\n",
            ORIGINAL_FINISH,
        ]
        .concat();
        let modified = delegate(&source).ok_or("SDL source rejected")?;
        require(modified.contains(MAIN_HOOK))?;
        require(modified.contains(OWNED_RUNNER))?;
        require(modified.contains(DELEGATE_CALL))?;
        require(modified.contains(GUARDED_FINISH))?;
        require(!modified.contains(ORIGINAL_RUNNER))?;
        require(!modified.contains(ORIGINAL_FINISH))?;
        require(modified.matches(ORIGINAL_CALL).count() == 1)?;
        require(modified.matches(MAIN_DECLARATION).count() == 1)?;
        require(delegate(&modified).is_none())?;
        require(adapt_activity(&source).is_none())?;
        for fragment in [
            MAIN_DECLARATION,
            ORIGINAL_RUNNER,
            ORIGINAL_CALL,
            ORIGINAL_FINISH,
        ] {
            require(delegate(&format!("{source}\n{fragment}")).is_none())?;
            require(delegate(&source.replace(fragment, "")).is_none())?;
        }
        Ok(())
    }

    fn require(condition: bool) -> Result<(), String> {
        condition
            .then_some(())
            .ok_or_else(|| "Android source contract failed".into())
    }

    #[test]
    fn title_manifest_has_one_debug_launcher_and_no_permissions()
    -> Result<(), String> {
        let manifest = manifest("simpsons-arcade").ok_or("invalid title")?;
        require(manifest.contains("org.games.title_simpsons_arcade"))?;
        require(manifest.contains("org.games.runtime.GamesActivity"))?;
        require(manifest.contains("android:launchMode=\"singleInstance\""))?;
        require(manifest.contains("android.intent.category.LAUNCHER"))?;
        require(manifest.contains("android:debuggable=\"true\""))?;
        require(!manifest.contains("uses-permission"))?;
        for invalid in ["", "..", "Title", "a/b", "a\"b", "a&b", "a_b"] {
            require(super::manifest(invalid).is_none())?;
        }
        Ok(())
    }
}
