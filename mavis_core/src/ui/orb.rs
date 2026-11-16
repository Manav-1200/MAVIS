// mavis_core/src/ui/orb.rs
// Living Orb UI. Small (80×80), borderless, transparent, draggable.
// Renders a soft pulsing circle that reacts to OrbState.
// Phase 5: Voice activity LED — brightness modulates with real-time VAD energy.
//
// On Wayland compositors with the layer shell (niri, Hyprland, Sway, KDE…)
// the orb is an overlay: above every window, on every workspace, never
// focused. Elsewhere (GNOME, X11) it's a borderless window.

use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::ui::states::OrbState;

const ORB_SIZE: usize = 80;
const BUFFER_LEN: usize = ORB_SIZE * ORB_SIZE;

/// A press shorter than this that barely moves is a tap, not a drag.
const TAP_MAX_HOLD: Duration = Duration::from_millis(400);
const TAP_MAX_TRAVEL: f32 = 4.0;

fn is_tap(held: Duration, travel: f32) -> bool {
    held <= TAP_MAX_HOLD && travel < TAP_MAX_TRAVEL
}

#[derive(Clone)]
pub struct Orb {
    state_tx: Sender<OrbState>,
    shutdown_tx: Sender<()>,
    energy_tx: Sender<f32>,
}

impl Orb {
    /// `on_tap` runs (on the orb's thread) when the orb is clicked
    /// without being dragged.
    pub fn new(on_tap: impl Fn() + Send + 'static) -> Self {
        let (state_tx, state_rx) = channel::<OrbState>();
        let (shutdown_tx, shutdown_rx) = channel::<()>();
        let (energy_tx, energy_rx) = channel::<f32>();

        // MAVIS_ORB=off runs headless. Useful on compositors where the orb
        // can't be placed sensibly — MAVIS still listens, thinks and speaks,
        // it just has no visible indicator.
        if matches!(std::env::var("MAVIS_ORB").as_deref(), Ok("off") | Ok("0")) {
            log::info!("Orb: disabled via MAVIS_ORB=off");
            // Drain the channels so senders never block on a full queue.
            thread::spawn(move || {
                loop {
                    let _ = state_rx.recv_timeout(Duration::from_secs(1));
                    while energy_rx.try_recv().is_ok() {}
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }
                }
            });
            return Self { state_tx, shutdown_tx, energy_tx };
        }

        thread::spawn(move || {
            // MAVIS_ORB=window keeps the plain window even where an overlay
            // is possible.
            let want_overlay = !matches!(std::env::var("MAVIS_ORB").as_deref(), Ok("window"));
            #[cfg(target_os = "linux")]
            if want_overlay {
                if let Some(overlay) = overlay::connect() {
                    log::info!("Orb: floating as an overlay");
                    overlay::run(overlay, state_rx, energy_rx, shutdown_rx, Box::new(on_tap));
                    log::info!("Orb: render thread exiting");
                    return;
                }
                log::info!("Orb: no overlay support here — using a window");
            }
            #[cfg(not(target_os = "linux"))]
            let _ = want_overlay;

            let mut window = match Window::new(
                "MAVIS",
                ORB_SIZE,
                ORB_SIZE,
                WindowOptions {
                    borderless: true,
                    transparency: true,
                    resize: false,
                    scale: minifb::Scale::X1,
                    ..WindowOptions::default()
                },
            ) {
                Ok(w) => w,
                Err(e) => {
                    log::error!("Orb: failed to create window: {}", e);
                    return;
                }
            };

            // MAVIS_ORB_POS=x,y places the orb at startup — e.g. "1750,60"
            // for the top-right corner. Dragging depends on the windowing
            // backend honouring set_position (X11/XWayland do; native
            // Wayland forbids a client positioning itself), so this gives a
            // way to park it out of the way regardless.
            if let Ok(pos) = std::env::var("MAVIS_ORB_POS") {
                match pos.split_once(',') {
                    Some((x, y)) => {
                        match (x.trim().parse::<isize>(), y.trim().parse::<isize>()) {
                            (Ok(x), Ok(y)) => {
                                window.set_position(x, y);
                                log::info!("Orb: positioned at {},{}", x, y);
                            }
                            _ => log::warn!("Orb: MAVIS_ORB_POS must be two integers, e.g. 1750,60"),
                        }
                    }
                    None => log::warn!("Orb: MAVIS_ORB_POS must be 'x,y', e.g. 1750,60"),
                }
            }

            window.limit_update_rate(Some(Duration::from_millis(16))); // ~60 FPS

            let mut buffer: Vec<u32> = vec![0; BUFFER_LEN];
            let mut current_state = OrbState::Idle;
            let mut current_energy = 0.0f32;
            let start = Instant::now();

            let mut is_dragging = false;
            let mut was_mouse_down = false;
            let mut drag_anchor: (f32, f32) = (0.0, 0.0);
            // How long the button has been held, and how far it has moved.
            let mut pressed_at = Instant::now();
            let mut travel = 0.0f32;

            while window.is_open() && !window.is_key_down(Key::Escape) {
                // Poll state updates
                while let Ok(s) = state_rx.try_recv() {
                    current_state = s;
                }

                // Poll energy updates from VAD
                while let Ok(e) = energy_rx.try_recv() {
                    // Peak-hold: new energy replaces only if louder
                    current_energy = current_energy.max(e);
                }
                // Exponential decay so the LED trails off smoothly
                current_energy *= 0.92;

                // Graceful shutdown
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }

                // Drag-to-move.
                //
                // MouseMode::Pass, not Clamp: Clamp restricts the reported
                // cursor to the window's own 80×80 bounds, so the moment the
                // pointer left the orb the reading saturated and the window
                // stopped tracking it — the "only moves a few pixels" bug.
                //
                // Note this only works where the backend honours
                // set_position (X11/XWayland). On native Wayland the
                // protocol forbids a client positioning its own window, so
                // placement is the compositor's job — use its own move
                // binding (typically Mod+drag) or a window rule instead.
                let mouse_down = window.get_mouse_down(MouseButton::Left);
                let mouse_pos = window.get_mouse_pos(MouseMode::Pass).unwrap_or((0.0, 0.0));

                if mouse_down && !was_mouse_down {
                    is_dragging = true;
                    drag_anchor = mouse_pos;
                    pressed_at = Instant::now();
                    travel = 0.0;
                } else if !mouse_down {
                    if was_mouse_down && is_tap(pressed_at.elapsed(), travel) {
                        on_tap();
                    }
                    is_dragging = false;
                }
                was_mouse_down = mouse_down;

                if is_dragging {
                    let win_pos = window.get_position();
                    // Move by how far the pointer has travelled since the
                    // grab, relative to where it grabbed.
                    let dx = mouse_pos.0 - drag_anchor.0;
                    let dy = mouse_pos.1 - drag_anchor.1;
                    if dx.abs() >= 1.0 || dy.abs() >= 1.0 {
                        // Summed: the window follows the pointer, so the
                        // offset from the anchor stays small while dragging.
                        travel += dx.abs().max(dy.abs());
                        let new_x = win_pos.0 + dx as isize;
                        let new_y = win_pos.1 + dy as isize;
                        window.set_position(new_x, new_y);
                    }
                }

                let elapsed = start.elapsed().as_secs_f32();
                render_orb(&mut buffer, elapsed, current_state, current_energy);

                if let Err(e) = window.update_with_buffer(&buffer, ORB_SIZE, ORB_SIZE) {
                    log::error!("Orb: render error: {}", e);
                    break;
                }
            }

            log::info!("Orb: render thread exiting");
        });

        Orb {
            state_tx,
            shutdown_tx,
            energy_tx,
        }
    }

    pub fn set_state(&self, state: OrbState) {
        let _ = self.state_tx.send(state);
    }

    pub fn set_energy(&self, energy: f32) {
        let _ = self.energy_tx.send(energy);
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

fn render_orb(buffer: &mut [u32], time: f32, state: OrbState, energy: f32) {
    for p in buffer.iter_mut() {
        *p = 0x00000000;
    }

    let cx = ORB_SIZE as f32 / 2.0;
    let cy = ORB_SIZE as f32 / 2.0;
    let base_radius = ORB_SIZE as f32 * 0.35;

    let pulse = match state {
        OrbState::Idle => 1.0 + 0.05 * (time * 1.5).sin(),
        OrbState::Listening => 1.0 + 0.15 * (time * 4.0).sin(),
        OrbState::Thinking => 1.0 + 0.10 * (time * 3.0).sin(),
        OrbState::Speaking => 1.0 + 0.12 * (time * 5.0).sin(),
        OrbState::Working => 1.0 + 0.08 * (time * 2.5).sin(),
        OrbState::Error => 1.0 + 0.20 * (time * 6.0).sin(),
        OrbState::Asleep => 1.0 + 0.02 * (time * 0.8).sin(),
        OrbState::Celebrating => 1.0 + 0.18 * (time * 6.0).sin(),
        OrbState::Asking => 1.0 + 0.14 * (time * 2.0).sin(),
    };

    let radius = base_radius * pulse;

    let (r, g, b) = match state {
        OrbState::Idle => (100, 180, 255),
        OrbState::Listening => (255, 100, 100),
        OrbState::Thinking => (255, 200, 50),
        OrbState::Speaking => (50, 255, 150),
        OrbState::Working => (200, 100, 255),
        OrbState::Error => (255, 50, 50),
        OrbState::Asleep => (80, 80, 120),
        OrbState::Celebrating => (255, 215, 0),
        // Orange: between thinking yellow and error red.
        OrbState::Asking => (255, 140, 0),
    };

    // Voice activity LED scaling per state
    let energy_scale = match state {
        OrbState::Idle => 0.20,
        OrbState::Listening => 0.50,
        OrbState::Thinking => 0.10,
        OrbState::Speaking => 0.10,
        OrbState::Working => 0.10,
        OrbState::Error => 0.20,
        OrbState::Asleep => 0.05,
        OrbState::Celebrating => 0.15,
        OrbState::Asking => 0.30,
    };

    // Normalize RMS energy (typical range 0.0–0.05) to 0.0–1.0
    let normalized = (energy * 20.0).min(1.0);
    let energy_boost = normalized * energy_scale;

    for y in 0..ORB_SIZE {
        for x in 0..ORB_SIZE {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist < radius + 2.0 {
                let edge = ((radius - dist) / 2.0).clamp(0.0, 1.0);
                let alpha = (edge * edge * (3.0 - 2.0 * edge) * 255.0) as u32;

                let inner = (dist / radius).clamp(0.0, 1.0);
                let base_brightness = 1.0 - inner * 0.4;
                let brightness = (base_brightness + energy_boost).min(1.4);

                let pr = (r as f32 * brightness) as u32;
                let pg = (g as f32 * brightness) as u32;
                let pb = (b as f32 * brightness) as u32;

                buffer[y * ORB_SIZE + x] = (alpha << 24) | (pr << 16) | (pg << 8) | pb;
            }
        }
    }
}

/// 0xAARRGGBB with straight alpha → premultiplied, which Wayland expects.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn premultiplied(p: u32) -> u32 {
    let a = p >> 24;
    let scale = |c: u32| (c * a + 127) / 255;
    (a << 24) | (scale((p >> 16) & 0xff) << 16) | (scale((p >> 8) & 0xff) << 8) | scale(p & 0xff)
}

// ---------------------------------------------------------------------------
// The orb as a Wayland layer-shell overlay
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod overlay {
    use super::{is_tap, premultiplied, render_orb, OrbState, BUFFER_LEN, ORB_SIZE};
    use smithay_client_toolkit::{
        compositor::{CompositorHandler, CompositorState, FrameCallbackData},
        delegate_registry,
        output::{OutputHandler, OutputState},
        reexports::client::{
            globals::{registry_queue_init, GlobalList},
            protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
            Connection, EventQueue, QueueHandle,
        },
        registry::{ProvidesRegistryState, RegistryState},
        registry_handlers,
        seat::{
            pointer::{PointerEvent, PointerEventKind, PointerHandler},
            Capability, SeatHandler, SeatState,
        },
        shell::{
            wlr_layer::{
                Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
                LayerSurfaceConfigure,
            },
            WaylandSurface,
        },
        shm::{slot::SlotPool, Shm, ShmHandler},
    };
    use std::sync::mpsc::Receiver;
    use std::time::Instant;

    const BTN_LEFT: u32 = 0x110;
    const SIZE: i32 = ORB_SIZE as i32;

    /// A Wayland connection that has everything an overlay needs.
    pub struct Connected {
        conn: Connection,
        globals: GlobalList,
        queue: EventQueue<OrbLayer>,
        compositor: CompositorState,
        layer_shell: LayerShell,
        shm: Shm,
    }

    /// Some when this is a Wayland session whose compositor has the layer
    /// shell; None sends the caller to the window.
    pub fn connect() -> Option<Connected> {
        std::env::var_os("WAYLAND_DISPLAY")?;
        let conn = Connection::connect_to_env().ok()?;
        let (globals, queue) = registry_queue_init::<OrbLayer>(&conn).ok()?;
        let qh = queue.handle();
        Some(Connected {
            compositor: CompositorState::bind(&globals, &qh).ok()?,
            layer_shell: LayerShell::bind(&globals, &qh).ok()?,
            shm: Shm::bind(&globals, &qh).ok()?,
            conn,
            globals,
            queue,
        })
    }

    /// Where the orb sits: MAVIS_ORB_POS=x,y from the top left, else near
    /// the top right corner.
    fn placement() -> (Anchor, i32, i32) {
        let from_env = std::env::var("MAVIS_ORB_POS").ok().and_then(|p| {
            let (x, y) = p.split_once(',')?;
            Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
        });
        match from_env {
            Some((x, y)) => (Anchor::TOP | Anchor::LEFT, y.max(0), x.max(0)),
            None => (Anchor::TOP | Anchor::RIGHT, 60, 20),
        }
    }

    struct Press {
        anchor: (f64, f64),
        at: Instant,
        travel: f32,
        // Where the pointer is now, in the orb's coordinates at the press.
        to: (f64, f64),
    }

    pub struct OrbLayer {
        registry_state: RegistryState,
        seat_state: SeatState,
        output_state: OutputState,
        shm: Shm,
        pool: SlotPool,
        layer: LayerSurface,
        pointer: Option<wl_pointer::WlPointer>,
        output: Option<wl_output::WlOutput>,
        configured: bool,
        exit: bool,
        // Distance from the top, and from the anchored side.
        top: i32,
        side: i32,
        from_left: bool,
        press: Option<Press>,
        on_tap: Box<dyn Fn() + Send>,
        state_rx: Receiver<OrbState>,
        energy_rx: Receiver<f32>,
        shutdown_rx: Receiver<()>,
        state: OrbState,
        energy: f32,
        start: Instant,
        pixels: Vec<u32>,
    }

    pub fn run(
        connected: Connected,
        state_rx: Receiver<OrbState>,
        energy_rx: Receiver<f32>,
        shutdown_rx: Receiver<()>,
        on_tap: Box<dyn Fn() + Send>,
    ) {
        let Connected { conn, globals, mut queue, compositor, layer_shell, shm } = connected;
        let qh = queue.handle();
        let surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("mavis-orb"), None);
        let (anchor, top, side) = placement();
        let from_left = anchor.contains(Anchor::LEFT);
        layer.set_anchor(anchor);
        layer.set_size(SIZE as u32, SIZE as u32);
        // Never the keyboard's: typing stays in the user's window.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        let mut orb = OrbLayer {
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            pool: match SlotPool::new(BUFFER_LEN * 4 * 2, &shm) {
                Ok(pool) => pool,
                Err(e) => {
                    log::error!("Orb: no shared memory for the overlay: {}", e);
                    return;
                }
            },
            shm,
            layer,
            pointer: None,
            output: None,
            configured: false,
            exit: false,
            top,
            side,
            from_left,
            press: None,
            on_tap,
            state_rx,
            energy_rx,
            shutdown_rx,
            state: OrbState::Idle,
            energy: 0.0,
            start: Instant::now(),
            pixels: vec![0; BUFFER_LEN],
        };
        orb.apply_margins();
        orb.layer.commit();

        while !orb.exit {
            if let Err(e) = queue.blocking_dispatch(&mut orb) {
                log::error!("Orb: overlay connection lost: {}", e);
                break;
            }
        }
        drop(conn);
    }

    impl OrbLayer {
        /// The logical size of the screen the orb is on, so it can't be
        /// dragged off it.
        fn screen(&self) -> (i32, i32) {
            self.output
                .as_ref()
                .and_then(|o| self.output_state.info(o))
                .and_then(|info| info.logical_size)
                .unwrap_or((i32::MAX, i32::MAX))
        }

        fn apply_margins(&self) {
            if self.from_left {
                self.layer.set_margin(self.top, 0, 0, self.side);
            } else {
                self.layer.set_margin(self.top, self.side, 0, 0);
            }
        }

        fn draw(&mut self, qh: &QueueHandle<Self>) {
            while let Ok(s) = self.state_rx.try_recv() {
                self.state = s;
            }
            while let Ok(e) = self.energy_rx.try_recv() {
                self.energy = self.energy.max(e);
            }
            self.energy *= 0.92;
            if self.shutdown_rx.try_recv().is_ok() {
                self.exit = true;
                return;
            }

            render_orb(&mut self.pixels, self.start.elapsed().as_secs_f32(), self.state, self.energy);
            let Ok((buffer, canvas)) = self.pool.create_buffer(SIZE, SIZE, SIZE * 4, wl_shm::Format::Argb8888)
            else {
                return;
            };
            for (chunk, p) in canvas.chunks_exact_mut(4).zip(&self.pixels) {
                chunk.copy_from_slice(&premultiplied(*p).to_le_bytes());
            }
            let surface = self.layer.wl_surface();
            surface.damage_buffer(0, 0, SIZE, SIZE);
            surface.frame(qh, FrameCallbackData(surface.clone()));
            if buffer.attach_to(surface).is_ok() {
                self.layer.commit();
            }
        }

        fn pointer(&mut self, event: &PointerEvent) {
            match event.kind {
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    self.press = Some(Press {
                        anchor: event.position,
                        at: Instant::now(),
                        travel: 0.0,
                        to: event.position,
                    });
                }
                PointerEventKind::Motion { .. } => {
                    if let Some(press) = self.press.as_mut() {
                        press.to = event.position;
                        let (dx, dy) = (press.to.0 - press.anchor.0, press.to.1 - press.anchor.1);
                        press.travel = press.travel.max(dx.abs().max(dy.abs()) as f32);
                    }
                }
                PointerEventKind::Release { button: BTN_LEFT, .. } => {
                    let Some(press) = self.press.take() else { return };
                    if is_tap(press.at.elapsed(), press.travel) {
                        (self.on_tap)();
                        return;
                    }
                    // Moved once, on release. Compositors differ on when a
                    // new margin takes effect, so pointer positions during a
                    // drag can't be trusted against a moving orb; against
                    // the orb where the press began they can.
                    let dx = (press.to.0 - press.anchor.0).round() as i32;
                    let dy = (press.to.1 - press.anchor.1).round() as i32;
                    let (width, height) = self.screen();
                    self.top = (self.top + dy).clamp(0, (height - SIZE).max(0));
                    let side = self.side + if self.from_left { dx } else { -dx };
                    self.side = side.clamp(0, (width - SIZE).max(0));
                    self.apply_margins();
                    self.layer.commit();
                }
                _ => {}
            }
        }
    }

    impl CompositorHandler for OrbLayer {
        fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
        fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
        fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
            self.draw(qh);
        }
        fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, output: &wl_output::WlOutput) {
            self.output = Some(output.clone());
        }
        fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    }

    impl OutputHandler for OrbLayer {
        fn output_state(&mut self) -> &mut OutputState {
            &mut self.output_state
        }
        fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
        fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
        fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    }

    impl LayerShellHandler for OrbLayer {
        fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
            self.exit = true;
        }
        fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
            // Drawing once starts the frame callbacks that keep it animated.
            if !self.configured {
                self.configured = true;
                self.draw(qh);
            }
        }
    }

    impl SeatHandler for OrbLayer {
        fn seat_state(&mut self) -> &mut SeatState {
            &mut self.seat_state
        }
        fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
        fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
            if capability == Capability::Pointer && self.pointer.is_none() {
                self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
            }
        }
        fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
            if capability == Capability::Pointer {
                if let Some(pointer) = self.pointer.take() {
                    pointer.release();
                }
            }
        }
        fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    }

    impl PointerHandler for OrbLayer {
        fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
            for event in events {
                if &event.surface == self.layer.wl_surface() {
                    self.pointer(event);
                }
            }
        }
    }

    impl ShmHandler for OrbLayer {
        fn shm_state(&mut self) -> &mut Shm {
            &mut self.shm
        }
    }

    delegate_registry!(OrbLayer);

    impl ProvidesRegistryState for OrbLayer {
        fn registry(&mut self) -> &mut RegistryState {
            &mut self.registry_state
        }
        registry_handlers![OutputState, SeatState];
    }

    smithay_client_toolkit::delegate_dispatch2!(OrbLayer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_premultiplied_for_wayland() {
        assert_eq!(premultiplied(0xff10_2030), 0xff10_2030);
        assert_eq!(premultiplied(0x0000_00ff) >> 24, 0);
        assert_eq!(premultiplied(0x0000_00ff) & 0xff, 0);
        assert_eq!(premultiplied(0x80ff_ffff), 0x8080_8080);
    }

    #[test]
    fn a_quick_still_click_is_a_tap() {
        assert!(is_tap(Duration::from_millis(120), 0.0));
        assert!(is_tap(Duration::from_millis(400), 3.0));
    }

    #[test]
    fn a_drag_or_a_long_press_is_not() {
        assert!(!is_tap(Duration::from_millis(120), 30.0), "moved");
        assert!(!is_tap(Duration::from_millis(900), 0.0), "held");
    }
}