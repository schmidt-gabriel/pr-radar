//! PR Radar — a menu-bar popover, a triage window and a timeline feed, all
//! reading one derived snapshot from a single polling module.

// Public so `examples/snapshot.rs` can run one poll headlessly — the same code
// path the app uses, printed instead of rendered.
pub mod derive;
pub mod github;
pub mod model;
pub mod poller;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use model::Feed;
use poller::{Config, Poller, SeenStore};

const FEED_EVENT: &str = "feed";

pub struct AppState {
    feed: Mutex<Feed>,
    config: Mutex<Config>,
    data_dir: PathBuf,
    /// Pinged by ⌘R, the tray menu, or the frontend to short-circuit the sleep.
    refresh: tokio::sync::Notify,
}

impl AppState {
    fn set_feed(&self, feed: Feed) {
        *self.feed.lock().unwrap() = feed;
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_feed(state: tauri::State<'_, Arc<AppState>>) -> Feed {
    state.feed.lock().unwrap().clone()
}

#[tauri::command]
fn refresh(state: tauri::State<'_, Arc<AppState>>) {
    state.refresh.notify_one();
}

#[tauri::command]
fn get_config(state: tauri::State<'_, Arc<AppState>>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_config(config: Config, state: tauri::State<'_, Arc<AppState>>) {
    config.save(&state.data_dir);
    *state.config.lock().unwrap() = config;
    state.refresh.notify_one();
}

#[tauri::command]
fn hide_popover(app: AppHandle) {
    if let Some(win) = app.get_webview_window("popover") {
        let _ = win.hide();
    }
}

/// Bring the triage window forward, optionally on a specific view.
#[tauri::command]
fn open_main(app: AppHandle, view: Option<String>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        if let Some(view) = view {
            let _ = win.emit("goto-view", view);
        }
    }
    if let Some(pop) = app.get_webview_window("popover") {
        let _ = pop.hide();
    }
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------

/// Place the popover directly under the tray icon, the way a native menu-bar
/// app does. Falls back to wherever the window already is if the trigger
/// carried no rect (global shortcut, menu item).
fn position_under_tray(win: &tauri::WebviewWindow, rect: Option<tauri::Rect>) {
    let Some(rect) = rect else { return };
    let scale = win.scale_factor().unwrap_or(1.0);

    let (icon_x, icon_y) = match rect.position {
        tauri::Position::Physical(p) => (p.x as f64, p.y as f64),
        tauri::Position::Logical(p) => (p.x * scale, p.y * scale),
    };
    let (icon_w, icon_h) = match rect.size {
        tauri::Size::Physical(s) => (s.width as f64, s.height as f64),
        tauri::Size::Logical(s) => (s.width * scale, s.height * scale),
    };

    let win_w = win
        .outer_size()
        .map(|s| s.width as f64)
        .unwrap_or(400.0 * scale);
    let x = icon_x + icon_w / 2.0 - win_w / 2.0;
    let y = icon_y + icon_h + 4.0 * scale;

    let _ = win.set_position(PhysicalPosition::new(x.max(8.0), y));
}

fn toggle_popover(app: &AppHandle, rect: Option<tauri::Rect>) {
    let Some(win) = app.get_webview_window("popover") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        position_under_tray(&win, rect);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// macOS renders the tray title next to the icon — a compact "how many things
/// are on fire" badge that is readable without opening anything.
fn update_tray_title(app: &AppHandle, feed: &Feed) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    let title = match feed {
        Feed::Ready(snap) => {
            let blocked = snap.mine_counts.blocked;
            let queue = snap.queue.len();
            if blocked > 0 {
                Some(format!("{blocked}!"))
            } else if queue > 0 {
                Some(queue.to_string())
            } else {
                None
            }
        }
        Feed::Error(_) => Some("!".to_string()),
        Feed::Loading => None,
    };
    let _ = tray.set_title(title.as_deref());
}

// ---------------------------------------------------------------------------
// Poll loop
// ---------------------------------------------------------------------------

fn publish(app: &AppHandle, state: &Arc<AppState>, feed: Feed) {
    state.set_feed(feed.clone());
    update_tray_title(app, &feed);
    let _ = app.emit(FEED_EVENT, &feed);
}

async fn poll_loop(app: AppHandle, state: Arc<AppState>) {
    let (mut seen, had_history) = SeenStore::load(&state.data_dir);
    // First ever launch: adopt whatever is already there rather than firing a
    // notification for every open PR at once.
    let mut seed_only = !had_history;

    let mut poller: Option<Poller> = None;

    loop {
        let config = state.config.lock().unwrap().clone();

        // (Re)connect lazily so a missing `gh` login surfaces in the UI as an
        // error the user can fix, then recovers on the next tick.
        let needs_connect = poller
            .as_ref()
            .map(|p| p.config.org != config.org)
            .unwrap_or(true);
        if needs_connect {
            match Poller::new(config.clone()).await {
                Ok(p) => poller = Some(p),
                Err(e) => {
                    publish(&app, &state, Feed::Error(format!("{e:#}")));
                    wait_for_next(&state, config.poll_seconds).await;
                    continue;
                }
            }
        }

        let result = poller
            .as_ref()
            .expect("poller connected above")
            .poll()
            .await;

        match result {
            Ok(snapshot) => {
                let mut fresh: Vec<model::Event> = Vec::new();
                for e in &snapshot.events {
                    if !poller::is_notifiable(e) {
                        continue;
                    }
                    if seen.insert(e.id.clone()) && !seed_only {
                        fresh.push(e.clone());
                    }
                }

                if state.config.lock().unwrap().notify {
                    // Oldest first so the most recent ends up on top of the stack.
                    for e in fresh.iter().rev().take(5) {
                        let (title, body) = poller::notification_body(e);
                        let _ = app.notification().builder().title(title).body(body).show();
                    }
                }

                SeenStore::save(&state.data_dir, &seen);
                seed_only = false;
                publish(&app, &state, Feed::Ready(Box::new(snapshot)));
            }
            Err(e) => {
                publish(&app, &state, Feed::Error(format!("{e:#}")));
                // A failed request often means the token went stale; rebuild the
                // client on the next pass.
                poller = None;
            }
        }

        let interval = state.config.lock().unwrap().poll_seconds;
        wait_for_next(&state, interval).await;
    }
}

/// Sleep until the interval elapses or someone asks for a refresh.
async fn wait_for_next(state: &Arc<AppState>, seconds: u64) {
    let sleep = tokio::time::sleep(Duration::from_secs(seconds.clamp(10, 3600)));
    tokio::pin!(sleep);
    tokio::select! {
        _ = &mut sleep => {}
        _ = state.refresh.notified() => {}
    }
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            get_feed,
            refresh,
            get_config,
            set_config,
            hide_popover,
            open_main,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            std::fs::create_dir_all(&data_dir).ok();

            let config = Config::load(&data_dir);
            config.save(&data_dir);

            let state = Arc::new(AppState {
                feed: Mutex::new(Feed::Loading),
                config: Mutex::new(config),
                data_dir,
                refresh: tokio::sync::Notify::new(),
            });
            app.manage(state.clone());

            // --- tray ---------------------------------------------------------
            let open_item = MenuItem::with_id(app, "open", "Open PR Radar", true, None::<&str>)?;
            let refresh_item =
                MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &open_item,
                    &refresh_item,
                    &PredefinedMenuItem::separator(app)?,
                    &quit_item,
                ],
            )?;

            let tray_state = state.clone();
            TrayIconBuilder::with_id("main")
                // A template image: black shapes plus alpha, which macOS
                // recolors for light and dark menu bars. tray-icon scales any
                // source to 18pt tall, so 36px is an exact 2x for Retina.
                // Regenerate with icons/make_tray_icon.py.
                .icon(tauri::include_image!("./icons/tray.png"))
                .icon_as_template(true)
                .tooltip("PR Radar")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "open" => open_main(app.clone(), None),
                    "refresh" => tray_state.refresh.notify_one(),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        toggle_popover(tray.app_handle(), Some(rect));
                    }
                })
                .build(app)?;

            // --- popover behaviour ---------------------------------------------
            if let Some(pop) = app.get_webview_window("popover") {
                let pop_handle = pop.clone();
                pop.on_window_event(move |event| {
                    // Dismiss on click-away, like every other menu-bar app.
                    if let WindowEvent::Focused(false) = event {
                        let _ = pop_handle.hide();
                    }
                });
            }

            // Closing the triage window leaves the tray app running.
            if let Some(main) = app.get_webview_window("main") {
                let main_handle = main.clone();
                main.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = main_handle.hide();
                    }
                });
            }

            // --- global shortcut -------------------------------------------------
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
                };

                let toggle = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyP);
                handle.plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, _shortcut, event| {
                            if event.state() == ShortcutState::Pressed {
                                toggle_popover(app, None);
                            }
                        })
                        .build(),
                )?;
                let _ = handle.global_shortcut().register(toggle);
            }

            // --- poller ---------------------------------------------------------
            tauri::async_runtime::spawn(poll_loop(handle, state));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running PR Radar");
}
