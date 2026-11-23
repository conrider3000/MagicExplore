use crate::config::HotkeySpec;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, SetWindowRgn,
    MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT,
    VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallNextHookEx, DispatchMessageW, GetClassNameW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId,
    IsWindowVisible, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW,
    ShowWindow, TranslateMessage, UnhookWindowsHookEx, EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE,
    HHOOK, HWND_TOPMOST, KBDLLHOOKSTRUCT, MSG, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNOACTIVATE, WH_KEYBOARD_LL, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT,
};

#[derive(Debug, Clone)]
pub enum HookEvent {
    ToggleBar,
    Captured(HotkeySpec),
    CaptureCancelled,
    AdminWindowFocused(isize),
}

static HOOK_EVENT_TX: OnceLock<mpsc::Sender<HookEvent>> = OnceLock::new();
static HOTKEY_PACKED: AtomicU64 = AtomicU64::new(0);
static CAPTURING_HOTKEY: AtomicBool = AtomicBool::new(false);
static SHOW_ADMIN_TOAST: AtomicBool = AtomicBool::new(true);
static TRIGGER_HELD: AtomicBool = AtomicBool::new(false);
static SWALLOWED_VK: AtomicU32 = AtomicU32::new(0);

fn pack_hotkey(hk: &HotkeySpec, enabled: bool) -> u64 {
    let mut val = hk.vk as u64;
    if hk.shift {
        val |= 1 << 32;
    }
    if hk.alt {
        val |= 1 << 33;
    }
    if hk.ctrl {
        val |= 1 << 34;
    }
    if hk.win {
        val |= 1 << 35;
    }
    if enabled {
        val |= 1 << 40;
    }
    val
}

fn unpack_hotkey(val: u64) -> (bool, HotkeySpec) {
    let enabled = (val & (1 << 40)) != 0;
    let hk = HotkeySpec {
        win: (val & (1 << 35)) != 0,
        ctrl: (val & (1 << 34)) != 0,
        alt: (val & (1 << 33)) != 0,
        shift: (val & (1 << 32)) != 0,
        vk: (val & 0xFFFF_FFFF) as u32,
    };
    (enabled, hk)
}

pub fn update_hook_hotkey(hk: &HotkeySpec, enabled: bool) {
    HOTKEY_PACKED.store(pack_hotkey(hk, enabled), Ordering::Relaxed);
}

pub fn set_capture_mode(active: bool) {
    CAPTURING_HOTKEY.store(active, Ordering::Relaxed);
}

pub fn set_admin_toast_enabled(enabled: bool) {
    SHOW_ADMIN_TOAST.store(enabled, Ordering::Relaxed);
}

pub fn start_hooks(tx: mpsc::Sender<HookEvent>, initial_hk: HotkeySpec, enabled: bool, admin_toast: bool) {
    let _ = HOOK_EVENT_TX.set(tx);
    update_hook_hotkey(&initial_hk, enabled);
    set_admin_toast_enabled(admin_toast);

    std::thread::Builder::new()
        .name("magic-explore-win32-hooks".to_string())
        .spawn(|| unsafe {
            let kb_hook: HHOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), None, 0)
                .unwrap_or_default();

            let fg_hook: HWINEVENTHOOK = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            if !kb_hook.is_invalid() {
                let _ = UnhookWindowsHookEx(kb_hook);
            }
            if !fg_hook.is_invalid() {
                let _ = UnhookWinEvent(fg_hook);
            }
        })
        .expect("Falha ao iniciar thread de hooks Win32");
}

#[inline]
unsafe fn is_vk_down(vk: VIRTUAL_KEY) -> bool {
    (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0
}

#[inline]
fn is_modifier_vk(vk: u32) -> bool {
    matches!(
        vk,
        0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5
    )
}

#[inline]
unsafe fn inject_dummy_key() {
    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0xE8),
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0xE8),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];
    let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

unsafe extern "system" fn keyboard_hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let kbd = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    if (kbd.flags.0 & 0x10) != 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let msg = wparam.0 as u32;
    let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
    let vk = kbd.vkCode;

    if is_modifier_vk(vk) {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let win = is_vk_down(VK_LWIN) || is_vk_down(VK_RWIN);
    let ctrl = is_vk_down(VK_CONTROL) || is_vk_down(VK_LCONTROL) || is_vk_down(VK_RCONTROL);
    let alt = is_vk_down(VK_MENU) || is_vk_down(VK_LMENU) || is_vk_down(VK_RMENU);
    let shift = is_vk_down(VK_SHIFT) || is_vk_down(VK_LSHIFT) || is_vk_down(VK_RSHIFT);

    if CAPTURING_HOTKEY.load(Ordering::Relaxed) {
        if is_down {
            if vk == VK_ESCAPE.0 as u32 && !win && !ctrl && !alt && !shift {
                CAPTURING_HOTKEY.store(false, Ordering::Relaxed);
                SWALLOWED_VK.store(vk, Ordering::Relaxed);
                if let Some(tx) = HOOK_EVENT_TX.get() {
                    let _ = tx.send(HookEvent::CaptureCancelled);
                }
                return LRESULT(1);
            }

            let is_fn_key = (0x70..=0x7B).contains(&vk);
            if win || ctrl || alt || is_fn_key {
                if win || alt {
                    inject_dummy_key();
                }
                CAPTURING_HOTKEY.store(false, Ordering::Relaxed);
                SWALLOWED_VK.store(vk, Ordering::Relaxed);
                if let Some(tx) = HOOK_EVENT_TX.get() {
                    let _ = tx.send(HookEvent::Captured(HotkeySpec {
                        win,
                        ctrl,
                        alt,
                        shift,
                        vk,
                    }));
                }
                return LRESULT(1);
            }
        } else if is_up && SWALLOWED_VK.load(Ordering::Relaxed) == vk {
            SWALLOWED_VK.store(0, Ordering::Relaxed);
            return LRESULT(1);
        }
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let (enabled, target) = unpack_hotkey(HOTKEY_PACKED.load(Ordering::Relaxed));
    if enabled && vk == target.vk {
        if is_down
            && win == target.win
            && ctrl == target.ctrl
            && alt == target.alt
            && shift == target.shift
        {
            if !TRIGGER_HELD.swap(true, Ordering::Relaxed) {
                if win || alt {
                    inject_dummy_key();
                }
                if let Some(tx) = HOOK_EVENT_TX.get() {
                    let _ = tx.send(HookEvent::ToggleBar);
                }
            }
            SWALLOWED_VK.store(vk, Ordering::Relaxed);
            return LRESULT(1);
        }

        if is_up {
            TRIGGER_HELD.store(false, Ordering::Relaxed);
            if SWALLOWED_VK.load(Ordering::Relaxed) == vk {
                SWALLOWED_VK.store(0, Ordering::Relaxed);
                return LRESULT(1);
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn win_event_proc(
    _h_win_event_hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND
        || id_object != 0
        || hwnd.0.is_null()
        || !SHOW_ADMIN_TOAST.load(Ordering::Relaxed)
    {
        return;
    }

    if !IsWindowVisible(hwnd).as_bool() {
        return;
    }

    let mut class_buf = [0u16; 128];
    let len = GetClassNameW(hwnd, &mut class_buf);
    if len > 0 {
        let class_name = String::from_utf16_lossy(&class_buf[..len as usize]);
        if matches!(
            class_name.as_str(),
            "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "Progman"
                | "WorkerW"
                | "NotifyIconOverflowWindow"
                | "TopLevelWindowForOverflowXamlIsland"
                | "Windows.UI.Core.CoreWindow"
                | "MultitaskingViewFrame"
                | "TaskListThumbnailWnd"
        ) {
            return;
        }
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 || pid == GetCurrentProcessId() {
        return;
    }

    if is_process_elevated(pid) {
        if let Some(tx) = HOOK_EVENT_TX.get() {
            let _ = tx.send(HookEvent::AdminWindowFocused(hwnd.0 as isize));
        }
    }
}

unsafe fn is_process_elevated(pid: u32) -> bool {
    let proc_res = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
    let h_proc = match proc_res {
        Ok(h) => h,
        Err(e) => return e.code() == ERROR_ACCESS_DENIED.to_hresult(),
    };

    let mut h_token = HANDLE::default();
    let token_res = OpenProcessToken(h_proc, TOKEN_QUERY, &mut h_token);
    let elevated = match token_res {
        Ok(()) => {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut ret_len: u32 = 0;
            let ok = GetTokenInformation(
                h_token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            );
            let _ = windows::Win32::Foundation::CloseHandle(h_token);
            ok.is_ok() && elevation.TokenIsElevated != 0
        }
        Err(e) => e.code() == ERROR_ACCESS_DENIED.to_hresult(),
    };

    let _ = windows::Win32::Foundation::CloseHandle(h_proc);
    elevated
}

pub fn apply_window_rounding(hwnd_raw: isize, radius: i32) {
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let pref = DWMWCP_ROUND;
        let dwm_res = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );

        if dwm_res.is_err() {
            let mut rc = RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_ok() {
                let w = rc.right - rc.left;
                let h = rc.bottom - rc.top;
                let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius * 2, radius * 2);
                if !rgn.is_invalid() {
                    let _ = SetWindowRgn(hwnd, rgn, true);
                }
            }
        }
    }
}

pub fn configure_toast_ex_styles(hwnd_raw: isize) {
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let new_ex = ex
            | (WS_EX_NOACTIVATE.0 as isize)
            | (WS_EX_TOOLWINDOW.0 as isize)
            | (WS_EX_TRANSPARENT.0 as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex);
    }
}

pub fn configure_backdrop_ex_styles(hwnd_raw: isize) {
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let new_ex = ex | (WS_EX_NOACTIVATE.0 as isize) | (WS_EX_TOOLWINDOW.0 as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex);
    }
}

pub fn show_toast_at_top(toast_hwnd_raw: isize, target_hwnd_raw: isize) {
    let toast_hwnd = HWND(toast_hwnd_raw as *mut _);
    let target_hwnd = HWND(target_hwnd_raw as *mut _);
    unsafe {
        let hmon = MonitorFromWindow(target_hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let mut rc = RECT::default();
            let _ = GetWindowRect(toast_hwnd, &mut rc);
            let w = (rc.right - rc.left).max(440);
            let work = mi.rcWork;
            let x = work.left + ((work.right - work.left) - w) / 2;
            let y = work.top + 16;
            let _ = SetWindowPos(
                toast_hwnd,
                HWND_TOPMOST,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        let _ = ShowWindow(toast_hwnd, SW_SHOWNOACTIVATE);
    }
}

pub fn hide_hwnd(hwnd_raw: isize) {
    unsafe {
        let _ = ShowWindow(HWND(hwnd_raw as *mut _), SW_HIDE);
    }
}

/// Positions `main_hwnd` horizontally centered on the active cursor monitor,
/// while anchoring its top `y` coordinate to the fixed 64px search bar center
/// (`anchor_bar_phys_h`) so expanding the dropdown list grows strictly downwards.
pub fn center_hwnd_on_active_monitor(main_hwnd_raw: isize, anchor_bar_phys_h: i32) {
    let hwnd = HWND(main_hwnd_raw as *mut _);
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let mut rc = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rc);
            let phys_w = (rc.right - rc.left).max(300);
            let bar_h = anchor_bar_phys_h.max(64);
            let work = mi.rcWork;
            let work_w = work.right - work.left;
            let work_h = work.bottom - work.top;

            let x = work.left + (work_w - phys_w) / 2;
            let y = work.top + (work_h - bar_h) / 2;
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

/// Covers the active monitor with the opaque backdrop window directly behind `main_hwnd`.
pub fn show_backdrop_behind_main(backdrop_hwnd_raw: isize, main_hwnd_raw: isize) {
    let backdrop_hwnd = HWND(backdrop_hwnd_raw as *mut _);
    let main_hwnd = HWND(main_hwnd_raw as *mut _);
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let mon = mi.rcMonitor;
            let w = mon.right - mon.left;
            let h = mon.bottom - mon.top;
            let _ = SetWindowPos(
                backdrop_hwnd,
                main_hwnd,
                mon.left,
                mon.top,
                w,
                h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        let _ = ShowWindow(backdrop_hwnd, SW_SHOWNOACTIVATE);
    }
}

pub fn force_foreground_focus(hwnd_raw: isize) {
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let fg = GetForegroundWindow();
        let fg_tid = if !fg.0.is_null() {
            GetWindowThreadProcessId(fg, None)
        } else {
            0
        };
        let cur_tid = GetCurrentThreadId();
        if fg_tid != 0 && fg_tid != cur_tid {
            let _ = AttachThreadInput(cur_tid, fg_tid, true);
            let _ = SetForegroundWindow(hwnd);
            let _ = BringWindowToTop(hwnd);
            let _ = AttachThreadInput(cur_tid, fg_tid, false);
        } else {
            let _ = SetForegroundWindow(hwnd);
            let _ = BringWindowToTop(hwnd);
        }
    }
}
