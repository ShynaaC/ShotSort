# ShotSort

A local desktop app that puts new screenshots into the session you are working on.

## Current scope

This milestone is only screenshot storage and active sessions. PDF tools, OCR, duplicate cleanup, cloud accounts, and automatic deletion are out of scope.

- Choose your existing screenshot source with a native folder dialog. ShotSort creates its own session storage automatically, or you can select a custom location.
- Create named assignment sessions, each with its own folder.
- Use **Quick session** to create an automatically named session without opening File Explorer or typing a name. Start it explicitly when ready; no files are deleted on close.
- Start, pause, and explicitly switch the active session.
- Route new PNG, JPG, JPEG, and WebP files into the active session.
- List screenshots, search filenames, show counts and storage totals, and open files or folders.
- Persist folder settings and sessions locally. The app always reopens paused.
- Prevent multiple ShotSort instances from routing the same files.

## Run on Windows

Prerequisites: Node.js/npm, Rust, Microsoft C++ build tools, and WebView2 (the Tauri Windows prerequisites).

Run these commands from this directory, which contains package.json:

```powershell
npm install
npm run tauri dev
```

The repository is inside an outer directory also named shotsort. If you are in that outer directory, enter the inner shotsort directory first.

`npm run dev` runs only the web interface. The browser cannot access the native screenshot storage commands and shows a notice instead of pretending to move files.

## First session

1. Choose a dedicated screenshot source, such as your actual Pictures/Screenshots folder. Check where your screenshot tool really saves files; it may use OneDrive.
2. Leave **Let ShotSort manage it** selected and save setup. No destination folder needs to be created manually. You can alternatively choose an existing, separate storage location.
3. Click **Quick session** for an automatic name and folder, or **New session** to choose a name.
4. Click **Start session**, then save a screenshot using your usual screenshot tool.
5. It should appear in the session after the file has remained unchanged for about two seconds.
6. Use **Pause** to stop routing. Selecting a session in the sidebar only changes the view; **Switch routing here** changes the destination.

Keep ShotSort open or minimized while working. Closing the window stops routing; system-tray/background-on-close support is not included.

## File safety and limits

- Existing source filenames at start/resume are left alone. The app watches one folder, non-recursively, and treats every newly arriving supported image in it as a screenshot.
- It does not capture clipboard-only screenshots; your screenshot tool must save a file.
- Pending files keep the session they were first observed in when you switch sessions. Pausing cancels pending transfers; those files stay in the source.
- Files are first written to temporary storage, flushed, and published without overwriting another file. Collisions get numbered names, such as Screenshot (1).png.
- The source is removed only after the destination is saved and the source still has the expected size and modification time. If removal fails, both copies are retained and a warning is shown.
- There is a short period with a temporary second copy, so transfers need enough free destination space. This is an organizer, not a compressor or disk cleaner.
- Missing/unwritable folders and transfer errors are reported. A failed transfer is retried at most three times. Unmoved files remain in the source for manual review.
- Settings are atomically saved to sessions.json in Tauri's app data directory. A damaged configuration is not silently overwritten.
- Managed session folders live in a sessions directory alongside that settings file. Quick sessions use normal persistent storage, not the OS temporary directory. They remain after restarting; no expiry or automatic deletion is enabled.
- Changing the storage root affects future sessions only. Existing session folders are not relocated.
- Moves in OneDrive or other synced folders can propagate to your cloud storage and other devices.
- Storage totals are logical file sizes, not guaranteed reclaimable disk space.

## Implementation

- React + TypeScript + Vite for the interface.
- Tauri 2 + Rust for native commands and file operations.
- notify for native folder events, with periodic reconciliation for missed events.
- JSON for this small amount of session metadata; no database server or additional runtime.
- No production browser mocks, external fonts, image services, or cloud backend.

## Verification

```powershell
npm run build
cd src-tauri
cargo fmt -- --check
cargo test --lib
```

The Rust suite covers real temporary-file transfers, existing-file preservation, pause/resume, session persistence, in-flight session switching, name collisions, incomplete writes, missing destinations, path validation, damaged settings, and the actual background watcher. It also checks automatic folder creation, quick-session persistence, old configurations, preserving existing session locations, and failed folder creation.

For repeatable interface checks, run the Vite dev server and open:

`http://127.0.0.1:1420/tests/ui-harness.html`

The clearly labelled development-only harness mocks IPC, never touches files, and is not included in the production build. It supports managed/custom folder setup, named and quick session creation, start/pause/switch, simulated incoming screenshots, and a failed-start scenario. Native behavior is verified separately by the Rust tests.

Before distributing an installer, manually check native folder dialogs, opening images in the OS viewer, and the workflow with your real screenshot tool in the desktop app.
