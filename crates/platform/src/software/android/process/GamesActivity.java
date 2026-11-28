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
//   - Android Activity handoff to the packaged Rust application.
// - Must-Not:
//   - Interpret title arguments, assets, settings or game simulation.
// - Allows:
//   - Inputs: Android private context and SDL application arguments.
//   - Outputs: Explicit JNI launch and failure observation.
//   - Side effects: Platform library loading and app directory selection.
// - Split-When:
//   - Android requires independent Activity integration.
// - Merge-When:
//   - An upstream safe Rust SDL entry provides the same handoff.
// - Summary:
//   - Minimal Java integration for SDL2 and the shared Rust entry.
// - Description:
//   - Calls the platform JNI export from SDL's application runner.
// - Usage:
//   - Package with the pinned SDL Activity delegation hook.
// - Defaults:
//   - Explicit application and library identity, no external storage.

package org.games.runtime;

import android.util.Log;
import org.libsdl.app.SDLActivity;

public class GamesActivity extends SDLActivity {
    private static native int runNative(
        String artifact, String dataParent, String[] arguments);

    @Override
    protected String[] getLibraries() {
        return new String[] {"SDL2", "games_application"};
    }

    @Override
    protected void runApplication(
        String library, String function, String[] arguments) {
        try {
            String parent = getDir("games", MODE_PRIVATE).getAbsolutePath();
            int result = runNative(library, parent, arguments);
            if (result != 0) {
                Log.e("Games", "Application returned " + result);
            }
        } catch (RuntimeException error) {
            Log.e("Games", "Application launch failed", error);
        }
    }
}
