#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apps;
mod config;
mod win32_hooks;

use apps::{
    index_all_apps, index_shortcuts_only, load_cached_apps, open_explorer_search, open_web_search,
    shell_open, AppEntry,
};
use config::{
    check_windows_conflict, load_config, save_config, set_autostart_enabled, AppConfig, HotkeySpec,
};
use serde::Serialize;
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, Size, State, WebviewWindow, WindowEvent};
use window_vibrancy::apply_acrylic;
use windows::core::HSTRING;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const EMBEDDED_APP_ICON: &[u8] = include_bytes!("../icons/128x128.png");

struct AppState {
    config: Mutex<AppConfig>,
    apps: Mutex<Vec<AppEntry>>,
    last_opened_ms: AtomicU64,
    toast_seq: AtomicU64,
}

#[derive(Serialize)]
struct StateResponse {
    config: AppConfig,
    hotkey_display: String,
    app_count: usize,
    needs_consent: bool,
}

#[derive(Clone, Serialize)]
struct HotkeyCapturedPayload {
    hotkey: HotkeySpec,
    display: String,
    blocked: bool,
    conflict_reason: Option<String>,
}

#[derive(Clone, Serialize)]
struct AdminToastPayload {
    message: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn get_hwnd(win: &WebviewWindow) -> Option<isize> {
    win.hwnd().ok().map(|h| h.0 as isize)
}

fn anchor_bar_physical_height(win: &WebviewWindow) -> i32 {
    let scale = win.scale_factor().unwrap_or(1.0);
    (64.0 * scale).round() as i32
}

fn sync_backdrop_window(app: &AppHandle, show: bool) {
    let Some(backdrop_win) = app.get_webview_window("backdrop") else {
        return;
    };
    let Some(backdrop_hwnd) = get_hwnd(&backdrop_win) else {
        return;
    };

    if show {
        if let Some(main_win) = app.get_webview_window("main") {
            if let Some(main_hwnd) = get_hwnd(&main_win) {
                win32_hooks::show_backdrop_behind_main(backdrop_hwnd, main_hwnd);
            }
        }
    } else {
        win32_hooks::hide_hwnd(backdrop_hwnd);
        let _ = backdrop_win.hide();
    }
}

fn show_main_bar(app: &AppHandle, open_settings: bool) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<AppState>();
    state.last_opened_ms.store(now_ms(), Ordering::Relaxed);

    let use_opaque = {
        let cfg = state.config.lock().unwrap();
        cfg.opaque_backdrop
    };

    let bar_phys_h = anchor_bar_physical_height(&win);
    if let Some(hwnd) = get_hwnd(&win) {
        win32_hooks::center_hwnd_on_active_monitor(hwnd, bar_phys_h);
    }

    let _ = win.show();

    if let Some(hwnd) = get_hwnd(&win) {
        win32_hooks::center_hwnd_on_active_monitor(hwnd, bar_phys_h);
        win32_hooks::apply_window_rounding(hwnd, 18);
    }

    sync_backdrop_window(app, use_opaque);

    if let Some(hwnd) = get_hwnd(&win) {
        win32_hooks::force_foreground_focus(hwnd);
    }
    let _ = win.set_focus();

    if open_settings {
        let _ = win.emit("open-settings", ());
    } else {
        let _ = win.emit("bar-opened", ());
    }
}

fn hide_main_bar(app: &AppHandle) {
    win32_hooks::set_capture_mode(false);
    sync_backdrop_window(app, false);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

fn toggle_main_bar(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        hide_main_bar(app);
    } else {
        show_main_bar(app, false);
    }
}

fn trigger_background_app_refresh(app: AppHandle) {
    std::thread::spawn(move || {
        let prev_icons: HashMap<String, String> = {
            let state = app.state::<AppState>();
            let guard = state.apps.lock().unwrap();
            guard
                .iter()
                .filter_map(|a| a.icon_data_url.clone().map(|ic| (a.launch_target.clone(), ic)))
                .collect()
        };

        let updated = index_all_apps(&prev_icons);
        {
            let state = app.state::<AppState>();
            let mut guard = state.apps.lock().unwrap();
            *guard = updated;
        }
        let _ = app.emit("apps-updated", ());
    });
}

#[tauri::command]
fn get_state(state: State<'_, AppState>) -> StateResponse {
    let cfg = state.config.lock().unwrap().clone();
    let app_count = state.apps.lock().unwrap().len();
    let hotkey_display = cfg.hotkey.display_string();
    let needs_consent = !cfg.consent_asked;
    StateResponse {
        config: cfg,
        hotkey_display,
        app_count,
        needs_consent,
    }
}

#[tauri::command]
fn get_apps(state: State<'_, AppState>) -> Vec<AppEntry> {
    state.apps.lock().unwrap().clone()
}

#[tauri::command]
fn refresh_apps(app: AppHandle, state: State<'_, AppState>) -> usize {
    let prev_icons: HashMap<String, String> = {
        let guard = state.apps.lock().unwrap();
        guard
            .iter()
            .filter_map(|a| a.icon_data_url.clone().map(|ic| (a.launch_target.clone(), ic)))
            .collect()
    };
    let updated = index_all_apps(&prev_icons);
    let len = updated.len();
    {
        let mut guard = state.apps.lock().unwrap();
        *guard = updated;
    }
    let _ = app.emit("apps-updated", ());
    len
}

#[tauri::command]
fn set_window_height(app: AppHandle, height: f64) {
    let clamped = height.clamp(64.0, 560.0);
    if let Some(win) = app.get_webview_window("main") {
        let bar_phys_h = anchor_bar_physical_height(&win);
        let _ = win.set_size(Size::Logical(LogicalSize::new(680.0, clamped)));
        if let Some(hwnd) = get_hwnd(&win) {
            win32_hooks::center_hwnd_on_active_monitor(hwnd, bar_phys_h);
            win32_hooks::apply_window_rounding(hwnd, 18);
        }
    }
}

#[tauri::command]
fn hide_main_window(app: AppHandle) {
    hide_main_bar(&app);
}

#[tauri::command]
fn launch_app(app: AppHandle, state: State<'_, AppState>, app_id: String) -> Result<(), String> {
    let target = {
        let guard = state.apps.lock().unwrap();
        guard
            .iter()
            .find(|a| a.id == app_id)
            .map(|a| a.launch_target.clone())
    };
    let Some(target) = target else {
        return Err("Aplicativo não encontrado no índice.".to_string());
    };
    hide_main_bar(&app);
    shell_open(&target)
}

#[tauri::command]
fn search_explorer(app: AppHandle, query: String) -> Result<(), String> {
    hide_main_bar(&app);
    open_explorer_search(&query)
}

#[tauri::command]
fn search_web(app: AppHandle, query: String) -> Result<(), String> {
    hide_main_bar(&app);
    open_web_search(&query)
}

#[tauri::command]
fn set_autostart(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    set_autostart_enabled(enabled)?;
    let mut cfg = state.config.lock().unwrap();
    cfg.autostart = enabled;
    save_config(&cfg)
}

#[tauri::command]
fn set_show_admin_toast(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    win32_hooks::set_admin_toast_enabled(enabled);
    let mut cfg = state.config.lock().unwrap();
    cfg.show_admin_toast = enabled;
    save_config(&cfg)
}

#[tauri::command]
fn set_opaque_backdrop(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.opaque_backdrop = enabled;
        save_config(&cfg)?;
    }
    if let Some(main_win) = app.get_webview_window("main") {
        if main_win.is_visible().unwrap_or(false) {
            sync_backdrop_window(&app, enabled);
            if let Some(hwnd) = get_hwnd(&main_win) {
                win32_hooks::force_foreground_focus(hwnd);
            }
            let _ = main_win.set_focus();
        }
    }
    Ok(())
}

#[tauri::command]
fn start_hotkey_capture() {
    win32_hooks::set_capture_mode(true);
}

#[tauri::command]
fn stop_hotkey_capture() {
    win32_hooks::set_capture_mode(false);
}

#[tauri::command]
fn apply_hotkey(
    state: State<'_, AppState>,
    hotkey: HotkeySpec,
    authorized: bool,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.hotkey = hotkey.clone();
    cfg.hotkey_authorized = authorized;
    cfg.consent_asked = true;
    save_config(&cfg)?;
    win32_hooks::update_hook_hotkey(&hotkey, authorized);
    Ok(())
}

#[tauri::command]
fn respond_initial_consent(
    state: State<'_, AppState>,
    authorize_win_space: bool,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.consent_asked = true;
    cfg.hotkey_authorized = true;
    cfg.hotkey = if authorize_win_space {
        HotkeySpec::default()
    } else {
        HotkeySpec {
            win: false,
            ctrl: false,
            alt: true,
            shift: false,
            vk: 0x20,
        }
    };
    save_config(&cfg)?;
    win32_hooks::update_hook_hotkey(&cfg.hotkey, true);
    Ok(())
}

fn terminate_previous_instances() {
    let my_pid = std::process::id();
    let filter = format!("PID ne {}", my_pid);
    for exe_name in ["Magic Explore.exe", "magic-explore.exe", "Magic_Explore_Portable.exe"] {
        let _ = Command::new("taskkill.exe")
            .args(["/F", "/FI", &filter, "/IM", exe_name])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
}

fn ensure_single_instance() -> bool {
    let name = HSTRING::from("Local\\MagicExploreSingleInstanceMutex");
    unsafe {
        let handle = CreateMutexW(None, true, &name);
        if handle.is_err() {
            return true;
        }
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn main() {
    terminate_previous_instances();
    if !ensure_single_instance() {
        return;
    }

    let is_autostart_launch = std::env::args().any(|a| a == "--autostart");
    let initial_cfg = load_config();
    let cached = load_cached_apps();
    let initial_apps = if cached.is_empty() {
        index_shortcuts_only(&HashMap::new())
    } else {
        cached
    };

    let state = AppState {
        config: Mutex::new(initial_cfg.clone()),
        apps: Mutex::new(initial_apps),
        last_opened_ms: AtomicU64::new(0),
        toast_seq: AtomicU64::new(0),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_state,
            get_apps,
            refresh_apps,
            set_window_height,
            hide_main_window,
            launch_app,
            search_explorer,
            search_web,
            set_autostart,
            set_show_admin_toast,
            set_opaque_backdrop,
            start_hotkey_capture,
            stop_hotkey_capture,
            apply_hotkey,
            respond_initial_consent
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                WindowEvent::Focused(false) => {
                    let app = window.app_handle();
                    let state = app.state::<AppState>();
                    let elapsed = now_ms().saturating_sub(state.last_opened_ms.load(Ordering::Relaxed));
                    if elapsed > 180 {
                        hide_main_bar(app);
                    }
                }
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    hide_main_bar(window.app_handle());
                }
                _ => {}
            }
        })
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let embedded_icon = Image::from_bytes(EMBEDDED_APP_ICON).ok();

            if let Some(main_win) = app.get_webview_window("main") {
                if let Some(ref ic) = embedded_icon {
                    let _ = main_win.set_icon(ic.clone());
                }
                let _ = apply_acrylic(&main_win, Some((20, 20, 25, 175)));
                if let Some(hwnd) = get_hwnd(&main_win) {
                    win32_hooks::apply_window_rounding(hwnd, 18);
                }
            }

            if let Some(backdrop_win) = app.get_webview_window("backdrop") {
                if let Some(hwnd) = get_hwnd(&backdrop_win) {
                    win32_hooks::configure_backdrop_ex_styles(hwnd);
                }
            }

            if let Some(toast_win) = app.get_webview_window("admin-toast") {
                if let Some(hwnd) = get_hwnd(&toast_win) {
                    win32_hooks::configure_toast_ex_styles(hwnd);
                    win32_hooks::apply_window_rounding(hwnd, 24);
                }
            }

            let open_item = MenuItem::with_id(app, "open", "Abrir Magic Explore", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Configurações", true, None::<&str>)?;
            let refresh_item = MenuItem::with_id(app, "refresh", "Atualizar lista de apps", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&open_item, &settings_item, &refresh_item, &sep, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&tray_menu)
                .tooltip("Magic Explore")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main_bar(app, false),
                    "settings" => show_main_bar(app, true),
                    "refresh" => trigger_background_app_refresh(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main_bar(tray.app_handle());
                    }
                });

            if let Some(ic) = embedded_icon.or_else(|| app.default_window_icon().cloned()) {
                tray_builder = tray_builder.icon(ic);
            }
            tray_builder.build(app)?;

            let (tx, rx) = mpsc::channel::<win32_hooks::HookEvent>();
            let hotkey_active = initial_cfg.consent_asked && initial_cfg.hotkey_authorized;
            win32_hooks::start_hooks(
                tx,
                initial_cfg.hotkey.clone(),
                hotkey_active,
                initial_cfg.show_admin_toast,
            );

            let hook_app = app_handle.clone();
            std::thread::Builder::new()
                .name("magic-explore-hook-dispatch".to_string())
                .spawn(move || {
                    let mut last_toast_global = Instant::now() - Duration::from_secs(10);
                    let mut last_toast_by_hwnd: HashMap<isize, Instant> = HashMap::new();

                    while let Ok(evt) = rx.recv() {
                        match evt {
                            win32_hooks::HookEvent::ToggleBar => {
                                toggle_main_bar(&hook_app);
                            }
                            win32_hooks::HookEvent::CaptureCancelled => {
                                let _ = hook_app.emit("hotkey-capture-cancelled", ());
                            }
                            win32_hooks::HookEvent::Captured(hk) => {
                                let (blocked, reason) = check_windows_conflict(&hk);
                                let payload = HotkeyCapturedPayload {
                                    display: hk.display_string(),
                                    hotkey: hk,
                                    blocked,
                                    conflict_reason: reason.map(|s| s.to_string()),
                                };
                                let _ = hook_app.emit("hotkey-captured", payload);
                            }
                            win32_hooks::HookEvent::AdminWindowFocused(target_hwnd) => {
                                let now = Instant::now();
                                if now.duration_since(last_toast_global) < Duration::from_millis(3200) {
                                    continue;
                                }
                                if let Some(prev) = last_toast_by_hwnd.get(&target_hwnd) {
                                    if now.duration_since(*prev) < Duration::from_secs(15) {
                                        continue;
                                    }
                                }
                                last_toast_global = now;
                                last_toast_by_hwnd.insert(target_hwnd, now);

                                if let Some(toast_win) = hook_app.get_webview_window("admin-toast") {
                                    if let Some(toast_hwnd) = get_hwnd(&toast_win) {
                                        let hk_label = {
                                            let st = hook_app.state::<AppState>();
                                            let g = st.config.lock().unwrap();
                                            g.hotkey.display_string()
                                        };
                                        win32_hooks::show_toast_at_top(toast_hwnd, target_hwnd);
                                        let _ = toast_win.emit(
                                            "admin-toast-show",
                                            AdminToastPayload {
                                                message: format!(
                                                    "{} pausado nesta janela",
                                                    hk_label
                                                ),
                                            },
                                        );

                                        let st = hook_app.state::<AppState>();
                                        let seq = st.toast_seq.fetch_add(1, Ordering::Relaxed) + 1;
                                        let hide_app = hook_app.clone();
                                        std::thread::spawn(move || {
                                            std::thread::sleep(Duration::from_millis(3150));
                                            let st = hide_app.state::<AppState>();
                                            if st.toast_seq.load(Ordering::Relaxed) == seq {
                                                if let Some(tw) = hide_app.get_webview_window("admin-toast") {
                                                    let _ = tw.emit("admin-toast-hide", ());
                                                    std::thread::sleep(Duration::from_millis(240));
                                                    if let Some(th) = get_hwnd(&tw) {
                                                        win32_hooks::hide_hwnd(th);
                                                    }
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    }
                })?;

            trigger_background_app_refresh(app_handle.clone());

            if !initial_cfg.consent_asked || !is_autostart_launch {
                show_main_bar(&app_handle, false);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Erro ao executar Magic Explore");
}
