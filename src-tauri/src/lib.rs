mod storage;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use storage::{DeletionPreview, Snapshot, Storage};
use tauri::{
    menu::{MenuBuilder, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

struct AppState(Arc<Mutex<Storage>>);

struct TrayState {
    tray: TrayIcon<Wry>,
    status: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    quitting: AtomicBool,
}

const TRAY_OPEN: &str = "tray-open";
const TRAY_STATUS: &str = "tray-status";
const TRAY_PAUSE: &str = "tray-pause";
const TRAY_QUIT: &str = "tray-quit";

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Storage>, String> {
    state
        .0
        .lock()
        .map_err(|_| "Screenshot storage is unavailable. Restart ShotSort.".into())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn sync_tray_status(app: &AppHandle, monitoring: bool) {
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    let label = if monitoring {
        "Routing: Active"
    } else {
        "Routing: Paused"
    };
    let _ = state.status.set_text(label);
    let _ = state.pause.set_enabled(monitoring);
    let _ = state.tray.set_tooltip(Some(format!("ShotSort — {label}")));
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, TRAY_STATUS, "Routing: Paused", false, None::<&str>)?;
    let open = MenuItem::with_id(app, TRAY_OPEN, "Open ShotSort", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, TRAY_PAUSE, "Pause routing", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT, "Quit ShotSort", true, None::<&str>)?;
    let menu = MenuBuilder::new(app)
        .item(&status)
        .separator()
        .item(&open)
        .item(&pause)
        .separator()
        .item(&quit)
        .build()?;
    let mut tray = TrayIconBuilder::with_id("shotsort-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("ShotSort — Routing: Paused")
        .on_menu_event(|app, event| match event.id().0.as_str() {
            TRAY_OPEN => show_main_window(app),
            TRAY_PAUSE => {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut storage) = state.0.lock() {
                        storage.pause();
                    }
                }
                sync_tray_status(app, false);
            }
            TRAY_QUIT => {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut storage) = state.0.lock() {
                        storage.pause();
                    }
                }
                if let Some(state) = app.try_state::<TrayState>() {
                    state.quitting.store(true, Ordering::SeqCst);
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    let tray = tray.build(app)?;
    app.manage(TrayState {
        tray,
        status,
        pause,
        quitting: AtomicBool::new(false),
    });
    Ok(())
}

#[tauri::command]
async fn get_state(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    selected_id: Option<String>,
    include_storage: Option<bool>,
) -> Result<Snapshot, String> {
    let mut storage = lock(&state)?;
    storage.reconcile_missing_managed_sessions()?;
    let snapshot =
        storage.snapshot_with_storage(selected_id.as_deref(), include_storage.unwrap_or(false));
    sync_tray_status(&app, snapshot.monitoring);
    Ok(snapshot)
}

#[tauri::command]
async fn configure_folders(
    state: tauri::State<'_, AppState>,
    source: String,
    destination: String,
) -> Result<(), String> {
    lock(&state)?.configure(&source, &destination)
}

#[tauri::command]
async fn create_session(state: tauri::State<'_, AppState>, name: String) -> Result<String, String> {
    lock(&state)?.create_session(&name)
}

#[tauri::command]
async fn start_session(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    lock(&state)?.start(&id)?;
    sync_tray_status(&app, true);
    Ok(())
}

#[tauri::command]
async fn create_quick_session(state: tauri::State<'_, AppState>) -> Result<String, String> {
    lock(&state)?.create_quick_session()
}

#[tauri::command]
async fn get_deletion_preview(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<DeletionPreview, String> {
    lock(&state)?.deletion_preview(&id)
}

#[tauri::command]
async fn delete_session(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let monitoring = {
        let mut storage = lock(&state)?;
        storage.delete_session(&id)?;
        storage.monitoring
    };
    sync_tray_status(&app, monitoring);
    Ok(())
}

#[tauri::command]
async fn pause_session(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    lock(&state)?.pause();
    sync_tray_status(&app, false);
    Ok(())
}

#[tauri::command]
async fn choose_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .map_err(|e| e.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn open_session_folder(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let folder = lock(&state)?.session(&id)?.folder.clone();
    app.opener()
        .open_path(folder.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_screenshot(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    session_id: String,
    name: String,
) -> Result<(), String> {
    let path = lock(&state)?.screenshot_path(&session_id, &name)?;
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

fn watch_screenshots(storage: Arc<Mutex<Storage>>) -> std::thread::JoinHandle<()> {
    let storage = Arc::downgrade(&storage);
    std::thread::spawn(move || {
        let (sender, receiver) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher: Option<RecommendedWatcher> = None;
        let mut watched_path: Option<PathBuf> = None;
        let mut last_scan = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(400));
            let Some(shared) = storage.upgrade() else {
                break;
            };
            let Ok(mut engine) = shared.lock() else {
                break;
            };
            if !engine.monitoring {
                watcher.take();
                watched_path = None;
                while receiver.try_recv().is_ok() {}
                continue;
            }
            let source = engine.config.source_dir.clone();
            let mut changed = false;
            if source != watched_path {
                watcher.take();
                let tx = sender.clone();
                let result = notify::recommended_watcher(move |event| {
                    let _ = tx.send(event);
                })
                .and_then(|mut next| {
                    if let Some(path) = source.as_ref() {
                        next.watch(path, RecursiveMode::NonRecursive)?;
                    }
                    Ok(next)
                });
                match result {
                    Ok(next) => {
                        watcher = Some(next);
                        watched_path = source;
                        changed = true;
                    }
                    Err(e) => {
                        engine.pause();
                        engine.last_error = Some(format!(
                            "Cannot watch the screenshot folder: {e}. Routing is paused."
                        ));
                        continue;
                    }
                }
            }
            while let Ok(event) = receiver.try_recv() {
                match event {
                    Ok(event) => {
                        if !matches!(event.kind, notify::EventKind::Access(_)) {
                            changed = true;
                        }
                    }
                    Err(e) => {
                        engine.pause();
                        engine.last_error =
                            Some(format!("Folder monitoring failed: {e}. Routing is paused."));
                    }
                }
            }
            // Reconcile periodically to cover missed native events, including OneDrive folders.
            if engine.monitoring && (changed || last_scan.elapsed() >= Duration::from_secs(5)) {
                if let Err(e) = engine.discover() {
                    engine.pause();
                    engine.last_error = Some(e);
                }
                last_scan = Instant::now();
            }
            engine.tick();
        }
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = app.path().app_data_dir()?.join("sessions.json");
            let storage = Storage::load(path).map_err(std::io::Error::other)?;
            let storage = Arc::new(Mutex::new(storage));
            app.manage(AppState(storage.clone()));
            setup_tray(app)?;
            let _ = watch_screenshots(storage);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let quitting = window
                    .app_handle()
                    .try_state::<TrayState>()
                    .is_some_and(|state| state.quitting.load(Ordering::SeqCst));
                if !quitting {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            configure_folders,
            create_session,
            create_quick_session,
            get_deletion_preview,
            delete_session,
            start_session,
            pause_session,
            choose_folder,
            open_session_folder,
            open_screenshot
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn background_watcher_routes_a_real_file_and_stops_with_its_owner() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("Screenshots");
        let destination = temp.path().join("Assignments");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("existing.png"), b"leave existing files").unwrap();
        let mut engine = Storage::load(temp.path().join("settings.json")).unwrap();
        engine
            .configure(source.to_str().unwrap(), destination.to_str().unwrap())
            .unwrap();
        let id = engine.create_session("Watcher integration").unwrap();
        let folder = engine.session(&id).unwrap().folder.clone();
        engine.start(&id).unwrap();
        let shared = Arc::new(Mutex::new(engine));
        let worker = watch_screenshots(shared.clone());
        fs::write(source.join("new.png"), b"native watcher screenshot").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !folder.join("new.png").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        // Wait for the worker's transfer to finish before checking source removal.
        let engine = shared.lock().unwrap();
        assert_eq!(
            fs::read(folder.join("new.png")).unwrap(),
            b"native watcher screenshot"
        );
        assert!(!source.join("new.png").exists());
        assert!(source.join("existing.png").exists());
        assert!(engine.last_error.is_none());
        drop(engine);
        drop(shared);
        worker.join().unwrap();
    }
}
