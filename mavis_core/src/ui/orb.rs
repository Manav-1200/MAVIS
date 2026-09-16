// mavis_core/src/ui/orb.rs
// Living Orb UI. Small (80×80), borderless, transparent, draggable.
// Renders a soft pulsing circle that reacts to OrbState.
// Phase 5: Voice activity LED — brightness modulates with real-time VAD energy.

use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::ui::states::OrbState;

const ORB_SIZE: usize = 80;
const BUFFER_LEN: usize = ORB_SIZE * ORB_SIZE;

#[derive(Clone)]
pub struct Orb {
    state_tx: Sender<OrbState>,
    shutdown_tx: Sender<()>,
    energy_tx: Sender<f32>,
}

impl Orb {
    pub fn new() -> Self {
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
                } else if !mouse_down {
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