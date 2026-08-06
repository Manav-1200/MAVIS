// mavis_core/src/ui/orb.rs
// Living Orb UI. Small (80×80), borderless, transparent, draggable.
// Renders a soft pulsing circle that reacts to OrbState.

use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::ui::states::OrbState;

const ORB_SIZE: usize = 80;
const BUFFER_LEN: usize = ORB_SIZE * ORB_SIZE;

pub struct Orb {
    state_tx: Sender<OrbState>,
    shutdown_tx: Sender<()>,
}

impl Orb {
    pub fn new() -> Self {
        let (state_tx, state_rx) = channel::<OrbState>();
        let (shutdown_tx, shutdown_rx) = channel::<()>();

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

            window.limit_update_rate(Some(Duration::from_millis(16))); // ~60 FPS

            let mut buffer: Vec<u32> = vec![0; BUFFER_LEN];
            let mut current_state = OrbState::Idle;
            let start = Instant::now();

            let mut is_dragging = false;
            let mut was_mouse_down = false;
            let mut drag_anchor: (f32, f32) = (0.0, 0.0);

            while window.is_open() && !window.is_key_down(Key::Escape) {
                // Poll state updates from the async runtime
                while let Ok(s) = state_rx.try_recv() {
                    current_state = s;
                }

                // Graceful shutdown signal
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }

                // --- Drag-to-move ---
                let mouse_down = window.get_mouse_down(MouseButton::Left);
                let mouse_pos = window.get_mouse_pos(MouseMode::Clamp).unwrap_or((0.0, 0.0));

                if mouse_down && !was_mouse_down {
                    is_dragging = true;
                    drag_anchor = mouse_pos;
                } else if !mouse_down {
                    is_dragging = false;
                }
                was_mouse_down = mouse_down;

                if is_dragging {
                    let win_pos = window.get_position();
                    // cursor_screen = window_pos + mouse_rel
                    let cursor_screen_x = win_pos.0 as f32 + mouse_pos.0;
                    let cursor_screen_y = win_pos.1 as f32 + mouse_pos.1;

                    let new_x = (cursor_screen_x - drag_anchor.0) as isize;
                    let new_y = (cursor_screen_y - drag_anchor.1) as isize;

                    if (new_x, new_y) != win_pos {
                        window.set_position(new_x, new_y);
                    }
                }

                // --- Render ---
                let elapsed = start.elapsed().as_secs_f32();
                render_orb(&mut buffer, elapsed, current_state);

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
        }
    }

    pub fn set_state(&self, state: OrbState) {
        let _ = self.state_tx.send(state);
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

/// Render a soft circular orb into an ARGB buffer.
fn render_orb(buffer: &mut [u32], time: f32, state: OrbState) {
    // Clear to fully transparent
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
    };

    for y in 0..ORB_SIZE {
        for x in 0..ORB_SIZE {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist < radius + 2.0 {
                let edge = ((radius - dist) / 2.0).clamp(0.0, 1.0);
                let alpha = (edge * edge * (3.0 - 2.0 * edge) * 255.0) as u32;

                let inner = (dist / radius).clamp(0.0, 1.0);
                let brightness = 1.0 - inner * 0.4;

                let pr = (r as f32 * brightness) as u32;
                let pg = (g as f32 * brightness) as u32;
                let pb = (b as f32 * brightness) as u32;

                buffer[y * ORB_SIZE + x] = (alpha << 24) | (pr << 16) | (pg << 8) | pb;
            }
        }
    }
}