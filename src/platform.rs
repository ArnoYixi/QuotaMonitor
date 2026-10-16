use eframe::egui;
use std::{path::Path, sync::mpsc::Sender};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Clone, Copy, Debug)]
pub enum TrayAction {
    Show,
    Hide,
    Refresh,
    Settings,
    Quit,
    Available(bool),
}
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
fn build_tray(tx: Sender<TrayAction>, ctx: egui::Context) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    let show = MenuItem::new("显示悬浮窗", true, None);
    let hide = MenuItem::new("隐藏悬浮窗", true, None);
    let refresh = MenuItem::new("刷新全部", true, None);
    let settings = MenuItem::new("设置", true, None);
    let quit = MenuItem::new("退出 QuotaMonitori", true, None);
    menu.append_items(&[
        &show,
        &hide,
        &refresh,
        &settings,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(|e| e.to_string())?;
    let ids = [
        show.id().clone(),
        hide.id().clone(),
        refresh.id().clone(),
        settings.id().clone(),
        quit.id().clone(),
    ];
    let event_tx = tx.clone();
    let event_ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(i) = ids.iter().position(|id| *id == event.id) {
            let _ = event_tx.send(
                [
                    TrayAction::Show,
                    TrayAction::Hide,
                    TrayAction::Refresh,
                    TrayAction::Settings,
                    TrayAction::Quit,
                ][i],
            );
            // A hidden/occluded eframe window may not run App::update on a
            // repaint alone. Wake the native window before consuming the queue.
            if i != 1 {
                wake_window();
                event_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
            event_ctx.request_repaint();
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
            wake_window();
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.request_repaint();
        }
    }));
    let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("QuotaMonitori · 额度监听")
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn build_tray(tx: Sender<TrayAction>, ctx: egui::Context) -> Result<TrayIcon, String> {
    TrayIconEvent::set_event_handler(Some(move |event| {
        let action = match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } => Some(TrayAction::Show),
            TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Down,
                ..
            } => native_tray_menu(),
            _ => None,
        };
        if let Some(action) = action {
            let _ = tx.send(action);
            if matches!(action, TrayAction::Hide) {
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                        main_window(),
                        windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE,
                    );
                }
            } else {
                wake_window();
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
            ctx.request_repaint();
        }
    }));
    let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_menu_on_left_click(false)
        .with_tooltip("QuotaMonitori · 额度监听")
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn native_tray_menu() -> Option<TrayAction> {
    use windows_sys::Win32::{
        Foundation::*, System::Threading::GetCurrentProcessId, UI::WindowsAndMessaging::*,
    };
    unsafe extern "system" fn visit(hwnd: HWND, output: LPARAM) -> i32 {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == GetCurrentProcessId() {
                let mut class = [0u16; 64];
                let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
                if String::from_utf16_lossy(&class[..n as usize]) == "tray_icon_app" {
                    *(output as *mut HWND) = hwnd;
                    return 0;
                }
            }
            1
        }
    }
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        for (id, label) in [
            (1, "显示悬浮窗"),
            (2, "隐藏悬浮窗"),
            (3, "刷新全部"),
            (4, "设置"),
            (5, "退出 QuotaMonitori"),
        ] {
            if id == 5 {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            }
            let text: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
            AppendMenuW(menu, MF_STRING, id, text.as_ptr());
        }
        let mut owner = main_window();
        EnumWindows(Some(visit), &mut owner as *mut HWND as isize);
        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        SetForegroundWindow(owner);
        // Return the selected ID directly: no WM_COMMAND/subclass forwarding
        // through a third-party hidden window is needed.
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
        match command {
            1 => Some(TrayAction::Show),
            2 => Some(TrayAction::Hide),
            3 => Some(TrayAction::Refresh),
            4 => Some(TrayAction::Settings),
            5 => Some(TrayAction::Quit),
            _ => None,
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
                if String::from_utf16_lossy(&title[..len as usize]).starts_with("QuotaMonitori ·")
                {
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

fn wake_window() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = main_window();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
            PostMessageW(hwnd, WM_NULL, 0, 0);
        }
    }
}

pub fn prepare_overlay() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = main_window();
        if hwnd.is_null() {
            return;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let wanted = (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
        if style != wanted {
            let visible = IsWindowVisible(hwnd) != 0;
            if visible {
                ShowWindow(hwnd, SW_HIDE);
            }
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
            if visible {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
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
    style: quotamonitori::appearance::Appearance,
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
    let fallback = quotamonitori::appearance::Appearance::fallback(!ctx.style().visuals.dark_mode);
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
                        quotamonitori::appearance::Appearance::from_samples(&samples, entry.style);
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
        let left = (rect.left() * scale - 2.0).floor().max(0.0) as i32;
        let top = (rect.top() * scale - 2.0).floor().max(0.0) as i32;
        let right = ((rect.right() * scale + 1.0).ceil() as i32).min(client.right - 1);
        let bottom = ((rect.bottom() * scale + 1.0).ceil() as i32).min(client.bottom - 1);
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
                    pixels.push([(c & 255) as u8, ((c >> 8) & 255) as u8, ((c >> 16) & 255) as u8]);
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
    pub fn start(tx: Sender<TrayAction>, ctx: egui::Context) -> Self {
        #[cfg(windows)]
        {
            let icon = build_tray(tx.clone(), ctx).ok();
            let _ = tx.send(TrayAction::Available(icon.is_some()));
            Self { _icon: icon }
        }
        #[cfg(target_os = "linux")]
        {
            std::thread::spawn(move || {
                if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() || gtk::init().is_err() {
                    let _ = tx.send(TrayAction::Available(false));
                    ctx.request_repaint();
                    return;
                }
                let icon = build_tray(tx.clone(), ctx.clone()).ok();
                let _ = tx.send(TrayAction::Available(icon.is_some()));
                ctx.request_repaint();
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
            key.set_value("QuotaMonitori", &format!("\"{}\"", exe.display()))
                .map_err(|_| "无法保存开机启动")?;
        } else {
            match key.delete_value("QuotaMonitori") {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("无法关闭开机启动".into()),
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let config = directories::BaseDirs::new()
            .ok_or("无法定位配置目录")?
            .config_dir()
            .join("autostart");
        let path = config.join("quotamonitori.desktop");
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
            std::fs::write(path,format!("[Desktop Entry]\nType=Application\nName=QuotaMonitori\nExec=\"{quoted}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n")).map_err(|_|"无法保存启动文件")?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(|_| "无法删除启动文件")?;
        }
    }
    Ok(())
}
pub fn show_error(message: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "QuotaMonitori".encode_utf16().chain(Some(0)).collect();
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
    eprintln!("QuotaMonitori: {message}");
}


