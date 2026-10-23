use crate::desktop::Wake;
use eframe::egui;
use std::{path::Path, sync::mpsc::Sender};
#[cfg(not(windows))]
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Clone, Copy, Debug)]
pub enum TrayAction {
    Show,
    Hide,
    Refresh,
    Settings,
    ToggleLock,
    Quit,
    Available(bool),
}
#[derive(Default)]
pub struct Tray {
    #[cfg(windows)]
    _icon: Option<TrayIcon>,
}
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut data = vec![0; (size * size * 4) as usize];
    let s = size as f32;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - s / 2.0;
            let dy = y as f32 + 0.5 - s / 2.0;
            let r = (dx * dx + dy * dy).sqrt() / s;
            let a = (dy.atan2(dx) + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
            let color = if (0.28..0.44).contains(&r) {
                if a < std::f32::consts::TAU * 0.78 {
                    [47, 182, 135, 255]
                } else {
                    [74, 84, 93, 255]
                }
            } else {
                [0, 0, 0, 0]
            };
            let i = ((y * size + x) * 4) as usize;
            data[i..i + 4].copy_from_slice(&color);
        }
    }
    data
}
#[cfg(not(windows))]
fn build_tray(tx: Sender<TrayAction>, wake: Wake) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    let show = MenuItem::new("显示悬浮窗", true, None);
    let hide = MenuItem::new("隐藏悬浮窗", true, None);
    let refresh = MenuItem::new("刷新全部", true, None);
    let settings = MenuItem::new("设置", true, None);
    let lock = MenuItem::new("锁定 / 解锁悬浮", true, None);
    let quit = MenuItem::new("退出 QuotaMonitor", true, None);
    menu.append_items(&[
        &show,
        &hide,
        &refresh,
        &settings,
        &lock,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(|e| e.to_string())?;
    let ids = [
        show.id().clone(),
        hide.id().clone(),
        refresh.id().clone(),
        settings.id().clone(),
        lock.id().clone(),
        quit.id().clone(),
    ];
    let event_tx = tx.clone();
    let event_wake = wake.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(i) = ids.iter().position(|id| *id == event.id) {
            let _ = event_tx.send(
                [
                    TrayAction::Show,
                    TrayAction::Hide,
                    TrayAction::Refresh,
                    TrayAction::Settings,
                    TrayAction::ToggleLock,
                    TrayAction::Quit,
                ][i],
            );
            event_wake();
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = tx.send(TrayAction::Show);
            wake();
        }
    }));
    let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("QuotaMonitor · 额度监听")
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn build_tray(tx: Sender<TrayAction>, wake: Wake) -> Result<TrayIcon, String> {
    TrayIconEvent::set_event_handler(Some(move |event| {
        let action = match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => Some(TrayAction::Show),
            TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Up,
                ..
            } => native_tray_menu(),
            _ => None,
        };
        if let Some(action) = action {
            let _ = tx.send(action);
            wake();
        }
    }));
    let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_menu_on_left_click(false)
        .with_tooltip("QuotaMonitor · 额度监听")
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())
}

static OVERLAY_LOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_overlay_locked(locked: bool) {
    OVERLAY_LOCKED.store(locked, std::sync::atomic::Ordering::Relaxed);
}

pub fn overlay_locked() -> bool {
    OVERLAY_LOCKED.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Copy)]
pub enum OverlayAction {
    Reset,
    Lock,
    Settings,
    Quit,
}

pub fn overlay_menu(can_reset: bool, can_lock: bool) -> Option<OverlayAction> {
    #[cfg(windows)]
    {
        match native_popup(
            &[
                (1, "使用重置卡", can_reset),
                (2, "锁定悬浮", can_lock),
                (3, "打开菜单", true),
                (4, "退出", true),
            ],
            false,
        ) {
            1 => Some(OverlayAction::Reset),
            2 => Some(OverlayAction::Lock),
            3 => Some(OverlayAction::Settings),
            4 => Some(OverlayAction::Quit),
            _ => None,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (can_reset, can_lock);
        None
    }
}

#[cfg(windows)]
fn native_tray_menu() -> Option<TrayAction> {
    let lock = if overlay_locked() {
        "解锁悬浮"
    } else {
        "锁定悬浮"
    };
    match native_popup(
        &[
            (1, "显示悬浮窗", true),
            (2, "隐藏悬浮窗", true),
            (3, "刷新全部", true),
            (4, "打开菜单", true),
            (6, lock, true),
            (0, "", false),
            (5, "退出", true),
        ],
        true,
    ) {
        1 => Some(TrayAction::Show),
        2 => Some(TrayAction::Hide),
        3 => Some(TrayAction::Refresh),
        4 => Some(TrayAction::Settings),
        5 => Some(TrayAction::Quit),
        6 => Some(TrayAction::ToggleLock),
        _ => None,
    }
}

#[cfg(windows)]
fn native_popup(items: &[(usize, &str, bool)], tray_owner: bool) -> i32 {
    use windows_sys::Win32::{Foundation::*, UI::WindowsAndMessaging::*};
    unsafe {
        // Both the locked overlay and tray-icon's hidden HWND are layered,
        // click-through windows. A popup needs an independently activatable
        // owner so mouse/keyboard input works even while the overlay is locked
        // or hidden. Never fall back to the locked overlay as the tray owner.
        let owner = if tray_owner {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                windows_sys::w!("STATIC"),
                windows_sys::w!("QuotaMonitor tray menu"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } else {
            main_window()
        };
        if owner.is_null() {
            return 0;
        }
        let menu = CreatePopupMenu();
        if menu.is_null() {
            if tray_owner {
                DestroyWindow(owner);
            }
            return 0;
        }
        for &(id, label, enabled) in items {
            if id == 0 {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            } else {
                let text: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                let flags = MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED };
                AppendMenuW(menu, flags, id, text.as_ptr());
            }
        }
        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        if tray_owner {
            ShowWindow(owner, SW_SHOWNOACTIVATE);
        }
        SetForegroundWindow(owner);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            owner,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(owner, WM_NULL, 0, 0);
        if tray_owner {
            DestroyWindow(owner);
        }
        command
    }
}

pub fn cancel_drag() {
    #[cfg(windows)]
    {
        DRAG_ORIGIN.set(None);
        unsafe {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture};
            if GetCapture() == main_window() {
                ReleaseCapture();
            }
        }
    }
}

#[cfg(all(test, windows))]
pub(crate) static WINDOW_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(all(test, windows))]
mod popup_tests {
    use super::*;
    use std::cell::Cell;
    use windows_sys::Win32::{
        Foundation::*, System::Threading::GetCurrentThreadId, UI::WindowsAndMessaging::*,
    };

    thread_local! {
        static TICKS: Cell<u32> = const { Cell::new(0) };
        static INTERACTIVE_OWNER: Cell<bool> = const { Cell::new(false) };
        static MENU_INPUT: Cell<char> = const { Cell::new('q') };
        static OWNER: Cell<isize> = const { Cell::new(0) };
    }

    unsafe extern "system" fn choose_menu_item(_: HWND, _: u32, timer: usize, _: u32) {
        unsafe {
            let ticks = TICKS.get() + 1;
            TICKS.set(ticks);
            if ticks > 20 {
                KillTimer(std::ptr::null_mut(), timer);
                EndMenu();
                return;
            }
            let mut info: GUITHREADINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            if GetGUIThreadInfo(GetCurrentThreadId(), &mut info) != 0
                && !info.hwndMenuOwner.is_null()
            {
                let style = GetWindowLongPtrW(info.hwndMenuOwner, GWL_EXSTYLE);
                OWNER.set(info.hwndMenuOwner as isize);
                INTERACTIVE_OWNER.set(
                    style & (WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE) as isize == 0,
                );
                // Exercise the real modal menu loop, targeting only this test's
                // owner window rather than injecting input into the desktop.
                if ticks == 1 {
                    let key = MENU_INPUT.get();
                    let message = if key == '\u{1b}' { WM_KEYDOWN } else { WM_CHAR };
                    PostMessageW(info.hwndMenuOwner, message, key as usize, 1);
                }
            }
        }
    }

    #[test]
    fn tray_popup_accepts_commands_with_a_hidden_click_through_overlay() {
        let _guard = WINDOW_TEST_LOCK.lock().unwrap();
        unsafe {
            let overlay = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                windows_sys::w!("STATIC"),
                windows_sys::w!("QuotaMonitor · popup regression"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!overlay.is_null());
            let tray = TrayIconBuilder::new().build().unwrap();
            let mut results = Vec::new();
            for (key, expected) in [('u', 1), ('\u{1b}', 0), ('q', 2)] {
                TICKS.set(0);
                INTERACTIVE_OWNER.set(false);
                OWNER.set(0);
                MENU_INPUT.set(key);
                let timer = SetTimer(std::ptr::null_mut(), 0, 100, Some(choose_menu_item));
                assert_ne!(timer, 0);
                let command = native_popup(&[(1, "&Unlock", true), (2, "&Quit", true)], true);
                KillTimer(std::ptr::null_mut(), timer);
                results.push((
                    command,
                    expected,
                    INTERACTIVE_OWNER.get(),
                    IsWindow(OWNER.get() as HWND) == 0,
                ));
            }
            let still_locked = GetWindowLongPtrW(overlay, GWL_EXSTYLE)
                & (WS_EX_LAYERED | WS_EX_TRANSPARENT) as isize
                == (WS_EX_LAYERED | WS_EX_TRANSPARENT) as isize;
            let still_hidden = IsWindowVisible(overlay) == 0;
            DestroyWindow(overlay);
            drop(tray);
            for (command, expected, interactive, destroyed) in results {
                assert_eq!(
                    command, expected,
                    "the popup must deliver its selected command"
                );
                assert!(interactive, "menu owner must accept input");
                assert!(destroyed, "the temporary menu owner must be destroyed");
            }
            assert!(
                still_locked && still_hidden,
                "opening the menu must preserve the overlay"
            );
        }
    }

    #[test]
    fn native_unlock_does_not_need_a_paint_or_visibility_toggle() {
        let _guard = WINDOW_TEST_LOCK.lock().unwrap();
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                windows_sys::w!("STATIC"),
                windows_sys::w!("input regression"),
                WS_POPUP,
                -30000,
                -30000,
                80,
                40,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect);
            for _ in 0..5 {
                sync_native_window(hwnd, Some(true), true, false);
                assert_ne!(
                    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TRANSPARENT as isize,
                    0
                );
                sync_native_window(hwnd, Some(true), false, false);
                assert_eq!(
                    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TRANSPARENT as isize,
                    0
                );
                assert_ne!(IsWindowVisible(hwnd), 0);
            }
            sync_native_window(hwnd, Some(false), true, false);
            sync_native_window(hwnd, Some(false), false, false);
            assert_eq!(
                IsWindowVisible(hwnd),
                0,
                "unlock must preserve hidden state"
            );
            assert_eq!(
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TRANSPARENT as isize,
                0
            );
            let mut after = RECT::default();
            GetWindowRect(hwnd, &mut after);
            assert_eq!(
                (rect.left, rect.top, rect.right, rect.bottom),
                (after.left, after.top, after.right, after.bottom)
            );
            DestroyWindow(hwnd);
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Edge {
    #[default]
    Free,
    Top,
    Bottom,
    Left,
    Right,
}
pub struct DesktopGeometry {
    pub window: egui::Rect,
    pub work: egui::Rect,
    pub dragging: bool,
}
impl DesktopGeometry {
    pub fn needs_recovery(&self) -> bool {
        !self.dragging && !self.work.contains_rect(self.window)
    }

    pub fn edge(&self, scale: f32) -> Edge {
        let gap = 40.0 * scale;
        // Crossing an edge must keep that edge selected, even when the
        // overshoot exceeds the docking threshold.
        let top = (self.window.top() - self.work.top()).max(0.0);
        let bottom = (self.work.bottom() - self.window.bottom()).max(0.0);
        if top.min(bottom) <= gap {
            if top <= bottom {
                Edge::Top
            } else {
                Edge::Bottom
            }
        } else if (self.window.left() - self.work.left()).max(0.0) <= gap {
            Edge::Left
        } else if (self.work.right() - self.window.right()).max(0.0) <= gap {
            Edge::Right
        } else {
            Edge::Free
        }
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    fn geometry(x: f32, y: f32) -> DesktopGeometry {
        DesktopGeometry {
            window: egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(542.0, 64.0)),
            work: egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1920.0, 1040.0)),
            dragging: false,
        }
    }

    #[test]
    fn unchanged_horizontal_window_recovers_after_top_overshoot() {
        for y in [-1.0, -12.0, -96.0, -200.0] {
            let g = geometry(200.0, y);
            assert!(g.needs_recovery());
            assert!(matches!(g.edge(1.0), Edge::Top));
            assert!(matches!(g.edge(1.5), Edge::Top));
        }
        assert!(!geometry(200.0, 0.0).needs_recovery());
        assert!(!geometry(200.0, 200.0).needs_recovery());
    }

    #[test]
    fn recovery_waits_for_drag_release() {
        let mut g = geometry(200.0, -12.0);
        g.dragging = true;
        assert!(!g.needs_recovery());
        g.dragging = false;
        assert!(g.needs_recovery());
    }

    #[test]
    fn recovery_uses_work_area_including_negative_monitor_origins() {
        let mut g = geometry(-1800.0, -100.0);
        g.work = egui::Rect::from_min_max(egui::pos2(-1920.0, -200.0), egui::pos2(0.0, 840.0));
        assert!(!g.needs_recovery());
        g.window = g.window.translate(egui::vec2(0.0, -101.0));
        assert!(g.needs_recovery());
        assert!(matches!(g.edge(1.0), Edge::Top));
    }

    #[test]
    fn other_edges_recover_without_a_size_change() {
        for (x, y, edge) in [
            (-100.0, 200.0, Edge::Left),
            (1500.0, 200.0, Edge::Right),
            (200.0, 1000.0, Edge::Bottom),
        ] {
            let g = geometry(x, y);
            assert!(g.needs_recovery());
            assert!(g.edge(1.0) == edge);
        }
    }
}

#[cfg(windows)]
fn main_window() -> windows_sys::Win32::Foundation::HWND {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::{
        Foundation::*, System::Threading::GetCurrentProcessId, UI::WindowsAndMessaging::*,
    };
    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    unsafe extern "system" fn visit(hwnd: HWND, result: LPARAM) -> i32 {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == GetCurrentProcessId() {
                let mut title = [0u16; 128];
                let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
                if String::from_utf16_lossy(&title[..len as usize]).starts_with("QuotaMonitor ·") {
                    *(result as *mut HWND) = hwnd;
                    return 0;
                }
            }
            1
        }
    }
    unsafe {
        let cached = WINDOW.load(Ordering::Relaxed) as HWND;
        if !cached.is_null() && IsWindow(cached) != 0 {
            return cached;
        }
        let mut hwnd = std::ptr::null_mut();
        EnumWindows(Some(visit), &mut hwnd as *mut HWND as isize);
        WINDOW.store(hwnd as isize, Ordering::Relaxed);
        hwnd
    }
}

#[derive(Default)]
pub struct WindowControl {
    #[cfg(not(windows))]
    last: Option<(Option<bool>, bool)>,
}

impl WindowControl {
    pub fn apply(
        &mut self,
        ctx: &egui::Context,
        visible: Option<bool>,
        passthrough: bool,
        reveal: bool,
    ) {
        #[cfg(windows)]
        {
            let _ = ctx;
            sync_native_window(main_window(), visible, passthrough, reveal);
        }
        #[cfg(not(windows))]
        {
            let state = (visible, passthrough);
            if self.last != Some(state) {
                if let Some(visible) = visible {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(passthrough));
                self.last = Some(state);
            }
            if reveal {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                if !passthrough {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
    }
}

#[cfg(windows)]
fn sync_native_window(
    hwnd: windows_sys::Win32::Foundation::HWND,
    visible: Option<bool>,
    passthrough: bool,
    reveal: bool,
) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        if hwnd.is_null() {
            return;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let mut wanted = (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
        let input_flags = (WS_EX_TRANSPARENT | WS_EX_LAYERED) as isize;
        if passthrough {
            wanted |= input_flags;
        } else {
            wanted &= !input_flags;
        }
        if style != wanted {
            // Windows input state belongs to the event loop, not a deferred
            // egui viewport command. Never hide/show to make a lock change stick.
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
        if let Some(visible) = visible {
            if !visible {
                if IsWindowVisible(hwnd) != 0 {
                    ShowWindow(hwnd, SW_HIDE);
                }
            } else if reveal || IsWindowVisible(hwnd) == 0 {
                ShowWindow(
                    hwnd,
                    if passthrough {
                        SW_SHOWNOACTIVATE
                    } else {
                        SW_RESTORE
                    },
                );
            }
            if visible && reveal && !passthrough {
                SetForegroundWindow(hwnd);
            }
        }
    }
}

#[cfg(windows)]
thread_local! {
    static DRAG_ORIGIN: std::cell::Cell<Option<[i32; 4]>> = const { std::cell::Cell::new(None) };
}

pub fn start_drag(ctx: &egui::Context) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::{POINT, RECT},
            UI::{Input::KeyboardAndMouse::SetCapture, WindowsAndMessaging::*},
        };
        let hwnd = main_window();
        if !hwnd.is_null() {
            let mut pointer = POINT { x: 0, y: 0 };
            let mut rect: RECT = std::mem::zeroed();
            if GetCursorPos(&mut pointer) != 0 && GetWindowRect(hwnd, &mut rect) != 0 {
                DRAG_ORIGIN.set(Some([pointer.x, pointer.y, rect.left, rect.top]));
                SetCapture(hwnd);
                ctx.request_repaint();
                return;
            }
        }
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
}

pub fn update_drag(ctx: &egui::Context) {
    #[cfg(windows)]
    if let Some([px, py, x, y]) = DRAG_ORIGIN.get() {
        unsafe {
            use windows_sys::Win32::{
                Foundation::POINT,
                UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
            };
            if GetAsyncKeyState(1) >= 0 {
                DRAG_ORIGIN.set(None);
                if GetCapture() == main_window() {
                    ReleaseCapture();
                }
                return;
            }
            let mut pointer = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pointer) != 0 {
                SetWindowPos(
                    main_window(),
                    std::ptr::null_mut(),
                    x + pointer.x - px,
                    y + pointer.y - py,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
    #[cfg(not(windows))]
    let _ = ctx;
}

pub fn desktop_geometry(ctx: &egui::Context) -> Option<DesktopGeometry> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
        let hwnd = main_window();
        if hwnd.is_null() {
            return None;
        }
        let mut rect: RECT = std::mem::zeroed();
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetWindowRect(hwnd, &mut rect) == 0
            || GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) == 0
        {
            return None;
        }
        let convert = |r: RECT| {
            egui::Rect::from_min_max(
                egui::pos2(r.left as f32, r.top as f32),
                egui::pos2(r.right as f32, r.bottom as f32),
            )
        };
        let _ = ctx;
        Some(DesktopGeometry {
            window: convert(rect),
            work: convert(info.rcWork),
            dragging: windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(1) < 0,
        })
    }
    #[cfg(not(windows))]
    ctx.input(|i| {
        Some(DesktopGeometry {
            window: i.viewport().outer_rect? * ctx.pixels_per_point(),
            work: egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                i.viewport().monitor_size? * ctx.pixels_per_point(),
            ),
            dragging: i.pointer.primary_down(),
        })
    })
}

pub fn resize_overlay(
    ctx: &egui::Context,
    size: egui::Vec2,
    edge: Edge,
    geometry: Option<&DesktopGeometry>,
) {
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    #[cfg(windows)]
    if let Some(g) = geometry {
        let size = size * ctx.pixels_per_point();
        let mut pos = g.window.min;
        match edge {
            Edge::Top => pos.y = g.work.top(),
            Edge::Bottom => pos.y = g.work.bottom() - size.y,
            Edge::Left => pos.x = g.work.left(),
            Edge::Right => pos.x = g.work.right() - size.x,
            Edge::Free => {}
        }
        pos.x = pos
            .x
            .clamp(g.work.left(), (g.work.right() - size.x).max(g.work.left()));
        pos.y = pos
            .y
            .clamp(g.work.top(), (g.work.bottom() - size.y).max(g.work.top()));
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let hwnd = main_window();
            if !hwnd.is_null() {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    pos.x.round() as i32,
                    pos.y.round() as i32,
                    size.x.round() as i32,
                    size.y.round() as i32,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (edge, geometry);
}

// Sample only the transparent gutter around each widget, never its own tint.
// One desktop blit per account/second avoids repeated compositor round trips.
#[cfg(windows)]
struct BackdropCache {
    sampled: std::time::Instant,
    frame: std::time::Instant,
    style: quotamonitor::appearance::Appearance,
    opacity: f32,
}
#[cfg(windows)]
thread_local! {
    static BACKDROPS: std::cell::RefCell<std::collections::HashMap<egui::Id, BackdropCache>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

pub fn overlay_appearance(
    ctx: &egui::Context,
    rect: egui::Rect,
    id: egui::Id,
) -> (egui::Color32, egui::Color32) {
    let fallback = quotamonitor::appearance::Appearance::fallback(!ctx.style().visuals.dark_mode);
    #[cfg(windows)]
    {
        return BACKDROPS.with_borrow_mut(|cache| {
            if cache.len() > 128 {
                cache.clear();
            }
            let now = std::time::Instant::now();
            let entry = cache.entry(id).or_insert_with(|| BackdropCache {
                sampled: now - std::time::Duration::from_secs(2),
                frame: now,
                style: fallback,
                opacity: 0.0,
            });
            if now.duration_since(entry.sampled).as_secs_f32() >= 1.0 {
                if let Some(samples) = sample_gutter(ctx, rect) {
                    let style =
                        quotamonitor::appearance::Appearance::from_samples(&samples, entry.style);
                    if style.opacity != entry.style.opacity {
                        entry.frame = now;
                    }
                    entry.style = style;
                }
                entry.sampled = now;
            }
            let target = f32::from(entry.style.opacity);
            let step = (now.duration_since(entry.frame).as_secs_f32() / 0.18).clamp(0.0, 1.0);
            entry.opacity += (target - entry.opacity) * step;
            if (target - entry.opacity).abs() < 1.0 {
                entry.opacity = target;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            entry.frame = now;
            let [r, g, b] = entry.style.tint();
            (
                if entry.style.dark_text {
                    egui::Color32::BLACK
                } else {
                    egui::Color32::WHITE
                },
                egui::Color32::from_rgba_unmultiplied(r, g, b, entry.opacity.round() as u8),
            )
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (rect, id);
        (
            if fallback.dark_text {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            },
            egui::Color32::TRANSPARENT,
        )
    }
}

#[cfg(windows)]
fn sample_gutter(ctx: &egui::Context, rect: egui::Rect) -> Option<Vec<[u8; 3]>> {
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        Graphics::Gdi::*,
        UI::WindowsAndMessaging::GetClientRect,
    };
    unsafe {
        let hwnd = main_window();
        if hwnd.is_null() {
            return None;
        }
        let mut origin = POINT { x: 0, y: 0 };
        let mut client: RECT = std::mem::zeroed();
        if ClientToScreen(hwnd, &mut origin) == 0 || GetClientRect(hwnd, &mut client) == 0 {
            return None;
        }
        let scale = ctx.pixels_per_point();
        let left = ((rect.left() - 2.0) * scale).floor().max(0.0) as i32;
        let top = ((rect.top() - 2.0) * scale).floor().max(0.0) as i32;
        let right = (((rect.right() + 1.0) * scale).ceil() as i32).min(client.right - 1);
        let bottom = (((rect.bottom() + 1.0) * scale).ceil() as i32).min(client.bottom - 1);
        let width = right - left + 1;
        let height = bottom - top + 1;
        if width < 4 || height < 4 {
            return None;
        }
        // Do not sample a clipped edge inside the widget's painted area.
        let top_clear = (top as f32) < rect.top() * scale - 0.5;
        let bottom_clear = (bottom as f32) > rect.bottom() * scale + 0.5;
        let left_clear = (left as f32) < rect.left() * scale - 0.5;
        let right_clear = (right as f32) > rect.right() * scale + 0.5;
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return None;
        }
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        if memory.is_null() || bitmap.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !memory.is_null() {
                DeleteDC(memory);
            }
            ReleaseDC(std::ptr::null_mut(), screen);
            return None;
        }
        let previous = SelectObject(memory, bitmap);
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            screen,
            origin.x + left,
            origin.y + top,
            SRCCOPY | CAPTUREBLT,
        );
        let mut pixels = Vec::with_capacity(80);
        if copied != 0 {
            let mut sample = |x, y| {
                let c = GetPixel(memory, x, y);
                if c != 0xffffffff {
                    pixels.push([
                        (c & 255) as u8,
                        ((c >> 8) & 255) as u8,
                        ((c >> 16) & 255) as u8,
                    ]);
                }
            };
            for i in 0..32 {
                let x = (width - 1) * i / 31;
                if top_clear {
                    sample(x, 0);
                }
                if bottom_clear {
                    sample(x, height - 1);
                }
            }
            for i in 0..8 {
                let y = (height - 1) * i / 7;
                if left_clear {
                    sample(0, y);
                }
                if right_clear {
                    sample(width - 1, y);
                }
            }
        }
        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);
        (pixels.len() >= 12).then_some(pixels)
    }
}

impl Tray {
    pub fn start(tx: Sender<TrayAction>, wake: Wake) -> Self {
        #[cfg(windows)]
        {
            let icon = build_tray(tx.clone(), wake.clone()).ok();
            let _ = tx.send(TrayAction::Available(icon.is_some()));
            wake();
            Self { _icon: icon }
        }
        #[cfg(target_os = "linux")]
        {
            std::thread::spawn(move || {
                if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() || gtk::init().is_err() {
                    let _ = tx.send(TrayAction::Available(false));
                    wake();
                    return;
                }
                let icon = build_tray(tx.clone(), wake.clone()).ok();
                let _ = tx.send(TrayAction::Available(icon.is_some()));
                wake();
                if icon.is_some() {
                    gtk::main();
                }
                drop(icon);
            });
            Self {}
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = tx.send(TrayAction::Available(false));
            wake();
            Self {}
        }
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        gtk::glib::MainContext::default().invoke(|| {
            if gtk::is_initialized() {
                gtk::main_quit();
            }
        });
    }
}

pub fn autostart(enabled: bool, exe: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::*};
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = root
            .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
            .map_err(|_| "无法更新开机启动设置")?;
        if enabled {
            key.set_value("QuotaMonitor", &format!("\"{}\"", exe.display()))
                .map_err(|_| "无法保存开机启动")?;
        } else {
            match key.delete_value("QuotaMonitor") {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("无法关闭开机启动".into()),
            }
        }
        match key.delete_value("QuotaMonitori") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("无法清理旧版开机启动".into()),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let config = directories::BaseDirs::new()
            .ok_or("无法定位配置目录")?
            .config_dir()
            .join("autostart");
        let path = config.join("quotamonitor.desktop");
        if enabled {
            std::fs::create_dir_all(&config).map_err(|_| "无法创建启动目录")?;
            let raw = exe.to_string_lossy();
            if raw.contains(['\n', '\r']) {
                return Err("程序路径无效".into());
            }
            let quoted = raw
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$")
                .replace('%', "%%");
            std::fs::write(path,format!("[Desktop Entry]\nType=Application\nName=QuotaMonitor\nExec=\"{quoted}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n")).map_err(|_|"无法保存启动文件")?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(|_| "无法删除启动文件")?;
        }
        let legacy = config.join("quotamonitori.desktop");
        if legacy.exists() {
            std::fs::remove_file(legacy).map_err(|_| "无法清理旧版开机启动".to_string())?;
        }
    }
    Ok(())
}
pub fn show_error(message: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "QuotaMonitor".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("QuotaMonitor: {message}");
}
