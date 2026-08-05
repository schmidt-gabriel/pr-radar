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
    /// Pinged by the refresh shortcut, the tray menu, or the frontend to
    /// short-circuit the sleep.
    refresh: tokio::sync::Notify,
    /// Tray menu line carrying the counts. macOS shows them in the tray title
    /// instead, which neither Linux nor Windows supports.
    status_item: Mutex<Option<MenuItem<tauri::Wry>>>,
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
    sync_dock_visibility(&app);
}

/// Show a Dock icon only while the triage window is open.
///
/// Closing that window leaves the app alive in the tray, and a tray-only app
/// with a Dock icon and no window is just clutter. `Accessory` also keeps the
/// popover from stealing focus the way a regular app would.
///
/// macOS only: Linux has no equivalent, and the window manager already hides
/// the entry when the window goes away.
fn sync_dock_visibility(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let window_open = app
            .get_webview_window("main")
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false);

        let policy = if window_open {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

// ---------------------------------------------------------------------------
// Tray
// ---------------------------------------------------------------------------

/// Anchor the popover to the tray icon, the way a native menu-bar app does.
///
/// The anchor flips when the icon sits in the lower half of the screen: a
/// bottom panel (the Windows taskbar, and KDE or Ubuntu docks by default) would
/// otherwise push the window straight off the bottom edge.
fn position_under_tray(win: &tauri::WebviewWindow, rect: Option<tauri::Rect>) {
    let Some(rect) = rect else {
        position_fallback(win);
        return;
    };
    let scale = win.scale_factor().unwrap_or(1.0);

    let (icon_x, icon_y) = match rect.position {
        tauri::Position::Physical(p) => (p.x as f64, p.y as f64),
        tauri::Position::Logical(p) => (p.x * scale, p.y * scale),
    };
    let (icon_w, icon_h) = match rect.size {
        tauri::Size::Physical(s) => (s.width as f64, s.height as f64),
        tauri::Size::Logical(s) => (s.width * scale, s.height * scale),
    };

    let (win_w, win_h) = win
        .outer_size()
        .map(|s| (s.width as f64, s.height as f64))
        .unwrap_or((400.0 * scale, 620.0 * scale));

    let gap = 4.0 * scale;
    let mut x = icon_x + icon_w / 2.0 - win_w / 2.0;

    // Screen the icon is on, so the flip test uses the right midpoint.
    let (screen_x, screen_y, screen_w, screen_h) = screen_bounds(win, scale);
    let below_midpoint = icon_y > screen_y + screen_h / 2.0;
    let y = if below_midpoint {
        icon_y - win_h - gap
    } else {
        icon_y + icon_h + gap
    };

    // Keep it on screen when the icon is near a corner.
    let max_x = screen_x + screen_w - win_w - 8.0;
    x = x.clamp(screen_x + 8.0, max_x.max(screen_x + 8.0));

    let _ = win.set_position(PhysicalPosition::new(x, y.max(screen_y)));
}

/// Where to put the popover when the trigger carried no icon rect: the global
/// shortcut on any platform, and every trigger under Linux appindicator, which
/// reports no geometry at all.
fn position_fallback(win: &tauri::WebviewWindow) {
    let scale = win.scale_factor().unwrap_or(1.0);
    let (screen_x, screen_y, screen_w, _) = screen_bounds(win, scale);
    let win_w = win
        .outer_size()
        .map(|s| s.width as f64)
        .unwrap_or(400.0 * scale);

    let margin = 12.0 * scale;
    let x = screen_x + screen_w - win_w - margin;
    // Clear a typical top panel rather than tucking under it.
    let y = screen_y + 32.0 * scale;

    let _ = win.set_position(PhysicalPosition::new(x.max(screen_x + margin), y));
}

fn screen_bounds(win: &tauri::WebviewWindow, scale: f64) -> (f64, f64, f64, f64) {
    match win.current_monitor() {
        Ok(Some(m)) => {
            let p = m.position();
            let s = m.size();
            (p.x as f64, p.y as f64, s.width as f64, s.height as f64)
        }
        // Assume a modest single screen rather than refusing to place at all.
        _ => (0.0, 0.0, 1440.0 * scale, 900.0 * scale),
    }
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

/// Compact badge for the macOS tray title: "how many things are on fire",
/// readable without opening anything.
fn badge_text(feed: &Feed) -> Option<String> {
    match feed {
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
    }
}

/// The same information spelled out, for surfaces that fit a sentence.
fn status_text(feed: &Feed) -> String {
    match feed {
        Feed::Ready(snap) => {
            let blocked = snap.mine_counts.blocked;
            let ready = snap.mine_counts.ready;
            let queue = snap.queue.len();

            let mut parts = Vec::new();
            if blocked > 0 {
                parts.push(format!("{blocked} blocked"));
            }
            if ready > 0 {
                parts.push(format!("{ready} ready to merge"));
            }
            if queue > 0 {
                parts.push(format!("{queue} to review"));
            }
            if parts.is_empty() {
                "Nothing waiting".to_string()
            } else {
                parts.join(" · ")
            }
        }
        Feed::Error(_) => "Cannot reach GitHub".to_string(),
        Feed::Loading => "Connecting…".to_string(),
    }
}

/// Surface the counts through whichever channels the host actually supports.
///
/// The three disagree, so all three get used:
/// - `set_title` works on macOS and Linux, but not Windows. On Linux a panel
///   may still truncate or hide it.
/// - `set_tooltip` works on macOS and Windows, but not Linux.
/// - The menu line works everywhere, and is the only channel guaranteed to be
///   readable under Linux appindicator.
fn update_tray_status(app: &AppHandle, state: &Arc<AppState>, feed: &Feed) {
    let status = status_text(feed);

    if let Some(tray) = app.tray_by_id("main") {
        if !cfg!(target_os = "windows") {
            let _ = tray.set_title(badge_text(feed).as_deref());
        }
        if !cfg!(target_os = "linux") {
            let _ = tray.set_tooltip(Some(&format!("PR Radar · {status}")));
        }
    }

    if let Some(item) = state.status_item.lock().unwrap().as_ref() {
        let _ = item.set_text(&status);
    }
}

// ---------------------------------------------------------------------------
// Poll loop
// ---------------------------------------------------------------------------

fn publish(app: &AppHandle, state: &Arc<AppState>, feed: Feed) {
    state.set_feed(feed.clone());
    update_tray_status(app, state, &feed);
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
                status_item: Mutex::new(None),
            });
            app.manage(state.clone());

            // --- tray ---------------------------------------------------------
            let status_item = MenuItem::with_id(app, "status", "Connecting…", false, None::<&str>)?;
            let open_item = MenuItem::with_id(app, "open", "Open PR Radar", true, None::<&str>)?;
            let refresh_item =
                MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let sep_top = PredefinedMenuItem::separator(app)?;
            let sep_bottom = PredefinedMenuItem::separator(app)?;

            // Clicking the tray icon already opens the popover, so a menu entry
            // for it is redundant. Linux is the exception: appindicator delivers
            // no click events, so there the entry is the only way in.
            #[cfg(target_os = "linux")]
            let popover_item =
                MenuItem::with_id(app, "popover", "Show popover", true, None::<&str>)?;

            let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
                vec![&status_item, &sep_top];
            #[cfg(target_os = "linux")]
            items.push(&popover_item);
            items.push(&open_item);
            items.push(&refresh_item);
            items.push(&sep_bottom);
            items.push(&quit_item);

            let menu = Menu::with_items(app, &items)?;
            *state.status_item.lock().unwrap() = Some(status_item.clone());

            // A template image is black shapes plus alpha, which macOS recolors
            // per menu-bar appearance. Elsewhere that is not a concept: the same
            // file renders as a black silhouette, invisible on a dark panel.
            // Only the path differs, so the builder chain below stays common and
            // therefore gets type-checked on every platform.
            // Regenerate both with icons/make_tray_icon.py.
            #[cfg(target_os = "macos")]
            let tray_icon = tauri::include_image!("./icons/tray.png");
            #[cfg(not(target_os = "macos"))]
            let tray_icon = tauri::include_image!("./icons/tray-color.png");

            let tray_state = state.clone();
            let tray = TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("PR Radar")
                .menu(&menu)
                // macOS opens the popover on left click and the menu on right.
                // Under appindicator there is no such distinction, so the menu
                // has to be the primary interaction.
                .show_menu_on_left_click(cfg!(target_os = "linux"))
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "popover" => toggle_popover(app, None),
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
                });

            tray.build(app)?;

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

            // Closing the triage window leaves the tray app running, so the
            // close button hides rather than quits, and the Dock icon goes with
            // it.
            if let Some(main) = app.get_webview_window("main") {
                let main_handle = main.clone();
                let dock_handle = app.handle().clone();
                main.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = main_handle.hide();
                        sync_dock_visibility(&dock_handle);
                    }
                });
            }

            // --- global shortcut -------------------------------------------------
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
                };

                // Cmd+Shift+P on macOS. Elsewhere SUPER is the Windows or Super
                // key, which desktop environments reserve heavily, so use
                // Ctrl+Alt+P instead.
                #[cfg(target_os = "macos")]
                let mods = Modifiers::SUPER | Modifiers::SHIFT;
                #[cfg(not(target_os = "macos"))]
                let mods = Modifiers::CONTROL | Modifiers::ALT;

                let toggle = Shortcut::new(Some(mods), Code::KeyP);
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
