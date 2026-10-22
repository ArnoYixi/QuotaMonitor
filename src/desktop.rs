use crate::ui::App;
use eframe::{UserEvent, egui};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::WindowId,
};

pub type Wake = Arc<dyn Fn() + Send + Sync>;
type Shared = Rc<RefCell<Option<(App, egui::Context)>>>;

/// eframe owns rendering; the application event loop owns commands and persistence.
/// In particular, a hidden window must not need a RedrawRequested event to quit.
pub fn run(
    options: eframe::NativeOptions,
    create: impl FnOnce(&eframe::CreationContext<'_>, Wake) -> App + 'static,
) -> Result<(), String> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let proxy = event_loop.create_proxy();
    let shared: Shared = Rc::default();
    let app_state = shared.clone();
    let renderer = eframe::create_native(
        "QuotaMonitori",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            // Send an event unconditionally. egui coalesces repaint requests and
            // may already have an outstanding repaint for a hidden window.
            let wake: Wake = Arc::new(move || {
                let _ = proxy.send_event(UserEvent::RequestRepaint {
                    viewport_id: egui::ViewportId::ROOT,
                    when: Instant::now(),
                    cumulative_pass_nr: ctx.cumulative_pass_nr(),
                });
            });
            *app_state.borrow_mut() = Some((create(cc, wake), cc.egui_ctx.clone()));
            Ok(Box::new(RenderApp(app_state)))
        }),
        &event_loop,
    );
    let mut desktop = Desktop { renderer, shared };
    event_loop.run_app(&mut desktop).map_err(|e| e.to_string())
}

struct RenderApp(Shared);
impl eframe::App for RenderApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some((app, _)) = self.0.borrow_mut().as_mut() {
            app.update(ctx, frame);
        }
    }
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some((mut app, _)) = self.0.borrow_mut().take() {
            app.on_exit(gl);
        }
    }
}

struct Desktop<R> {
    renderer: R,
    shared: Shared,
}
impl<R: ApplicationHandler<UserEvent>> Desktop<R> {
    fn poll(&mut self, event_loop: &ActiveEventLoop) {
        if let Some((app, ctx)) = self.shared.borrow_mut().as_mut() {
            app.poll(ctx);
            app.sync_window(ctx);
            if app.quitting() {
                event_loop.exit();
            }
        }
    }
}
impl<R: ApplicationHandler<UserEvent>> ApplicationHandler<UserEvent> for Desktop<R> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.renderer.resumed(event_loop);
        self.poll(event_loop);
    }
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.renderer.new_events(event_loop, cause);
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        // Consume commands even if eframe considers this repaint out of date.
        self.poll(event_loop);
        self.renderer.user_event(event_loop, event);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            if let Some((app, _)) = self.shared.borrow_mut().as_mut() {
                app.close_requested();
            }
            self.poll(event_loop);
            return;
        }
        self.renderer.window_event(event_loop, id, event);
        self.poll(event_loop);
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.renderer.device_event(event_loop, id, event);
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.renderer.about_to_wait(event_loop);
        self.poll(event_loop);
        // Autosave and demo timers continue even with no visible/paintable window.
        let deadline = Instant::now() + Duration::from_secs(1);
        let hidden = self
            .shared
            .borrow()
            .as_ref()
            .is_some_and(|(app, _)| !app.is_visible());
        if hidden {
            // A hidden HWND may never deliver the redraw eframe is polling for.
            // UserEvent still wakes this wait immediately when a command arrives.
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            return;
        }
        match event_loop.control_flow() {
            ControlFlow::Wait => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            ControlFlow::WaitUntil(when) if when > deadline => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline))
            }
            _ => {}
        }
    }
    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.renderer.suspended(event_loop);
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.renderer.exiting(event_loop);
    }
    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.renderer.memory_warning(event_loop);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::platform::{self, TrayAction};
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use winit::platform::windows::EventLoopBuilderExtWindows;

    // Use the real eframe renderer, then suppress its window events to simulate
    // a window that can no longer repaint. Commands must still reach the OS.
    struct PausableRenderer<'a> {
        inner: eframe::EframeWinitApplication<'a>,
        paused: Arc<AtomicBool>,
        exited: Arc<AtomicBool>,
        deadline: Instant,
    }
    impl ApplicationHandler<UserEvent> for PausableRenderer<'_> {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            self.inner.resumed(el);
        }
        fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
            self.inner.new_events(el, cause);
        }
        fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
            self.inner.user_event(el, event);
        }
        fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            if !self.paused.load(Ordering::SeqCst) {
                self.inner.window_event(el, id, event);
            }
        }
        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            self.inner.about_to_wait(el);
            if Instant::now() > self.deadline {
                el.exit();
            }
        }
        fn exiting(&mut self, el: &ActiveEventLoop) {
            self.inner.exiting(el);
            self.exited.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    #[ignore = "requires a Windows desktop and OpenGL; run with --ignored --exact"]
    fn event_loop_unlocks_and_exits_when_rendering_is_disabled() {
        let _guard = platform::WINDOW_TEST_LOCK.lock().unwrap();
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .with_any_thread(true)
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join(format!("quota-no-paint-{}", uuid::Uuid::new_v4()));
        let shared: Shared = Rc::default();
        let app_state = shared.clone();
        let app_dir = dir.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let inner = eframe::create_native(
            "QuotaMonitori regression",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_title("QuotaMonitori · no-paint regression")
                    .with_decorations(false)
                    .with_transparent(true)
                    .with_taskbar(false),
                renderer: eframe::Renderer::Glow,
                ..Default::default()
            },
            Box::new(move |cc| {
                let ctx = cc.egui_ctx.clone();
                let (app, tx) = crate::ui::layout_tests::control_fixture(&ctx, app_dir);
                *app_state.borrow_mut() = Some((app, ctx.clone()));
                ready_tx.send((tx, ctx)).unwrap();
                Ok(Box::new(RenderApp(app_state)))
            }),
            &event_loop,
        );
        let paused = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let renderer = PausableRenderer {
            inner,
            paused: paused.clone(),
            exited: exited.clone(),
            deadline: Instant::now() + Duration::from_secs(20),
        };
        let mut desktop = Desktop { renderer, shared };
        let proxy = event_loop.create_proxy();
        let probe = std::thread::spawn(move || {
            let (tx, ctx) = ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while ctx.cumulative_pass_nr() < 3 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            paused.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            let passes_before = ctx.cumulative_pass_nr();
            let handle = unsafe {
                FindWindowW(
                    std::ptr::null(),
                    windows_sys::w!("QuotaMonitori · no-paint regression"),
                )
            } as isize;
            let check = |action, locked, visible| {
                tx.send(action).unwrap();
                proxy
                    .send_event(UserEvent::RequestRepaint {
                        viewport_id: egui::ViewportId::ROOT,
                        cumulative_pass_nr: ctx.cumulative_pass_nr(),
                        when: Instant::now(),
                    })
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    let (actual_lock, actual_visible) = unsafe {
                        (
                            GetWindowLongPtrW(handle as _, GWL_EXSTYLE)
                                & WS_EX_TRANSPARENT as isize
                                != 0,
                            IsWindowVisible(handle as _) != 0,
                        )
                    };
                    if (actual_lock, actual_visible) == (locked, visible) {
                        return true;
                    }
                    if Instant::now() > deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            };
            let checks = [
                check(TrayAction::ToggleLock, true, true),
                check(TrayAction::ToggleLock, false, true),
                check(TrayAction::ToggleLock, true, true),
                check(TrayAction::Hide, true, false),
                check(TrayAction::ToggleLock, false, false),
                check(TrayAction::Settings, false, true),
                check(TrayAction::Hide, false, false),
            ];
            let passes_after = ctx.cumulative_pass_nr();
            paused.store(false, Ordering::SeqCst);
            let shown = check(TrayAction::Show, false, true);
            let deadline = Instant::now() + Duration::from_secs(3);
            while ctx.cumulative_pass_nr() <= passes_after && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let resumed = ctx.cumulative_pass_nr() > passes_after;
            let unlocked_after_paint = unsafe {
                GetWindowLongPtrW(handle as _, GWL_EXSTYLE) & WS_EX_TRANSPARENT as isize == 0
            };
            let hidden = check(TrayAction::Hide, false, false);
            tx.send(TrayAction::Quit).unwrap();
            proxy
                .send_event(UserEvent::RequestRepaint {
                    viewport_id: egui::ViewportId::ROOT,
                    cumulative_pass_nr: ctx.cumulative_pass_nr(),
                    when: Instant::now(),
                })
                .unwrap();
            (
                checks,
                passes_before,
                passes_after,
                shown && resumed && unlocked_after_paint && hidden,
            )
        });
        event_loop.run_app(&mut desktop).unwrap();
        let (checks, passes_before, passes_after, resumed) = probe.join().unwrap();
        assert!(
            checks.iter().all(|ok| *ok),
            "commands with painting disabled: {checks:?}"
        );
        assert!(exited.load(Ordering::SeqCst), "normal shutdown must run");
        assert!(passes_before >= 3, "real renderer must start successfully");
        assert_eq!(
            passes_before, passes_after,
            "commands must not need a UI pass"
        );
        assert!(
            resumed,
            "resuming painting must not restore a stale lock or prevent hidden quit"
        );
        assert!(
            !quotamonitori::store::load_settings(&dir)
                .unwrap()
                .lock_position
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
