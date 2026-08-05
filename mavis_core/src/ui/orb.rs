use crate::ui::states::OrbState;
use log::warn;
use minifb::{Scale, Window, WindowOptions};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

const ORB_SIZE: usize = 256;
const FRAME_TIME_MS: u64 = 16;

pub struct Orb {
    state_tx: mpsc::UnboundedSender<OrbState>,
    shutdown: Arc<AtomicBool>,
}

impl Orb {
    pub fn new() -> Self {
        let (state_tx, state_rx) = mpsc::unbounded_channel::<OrbState>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        std::thread::spawn(move || {
            if let Err(e) = run_render_loop(state_rx, shutdown_clone) {
                warn!("Orb render thread error: {}", e);
            }
        });

        Self { state_tx, shutdown }
    }

    pub fn set_state(&self, state: OrbState) {
        let _ = self.state_tx.send(state);
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn run_render_loop(
    mut state_rx: mpsc::UnboundedReceiver<OrbState>,
    shutdown: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let mut window = Window::new(
        "MAVIS",
        ORB_SIZE,
        ORB_SIZE,
        WindowOptions {
            borderless: true,
            transparency: true,
            topmost: true,
            resize: false,
            scale: Scale::X1,
            ..WindowOptions::default()
        },
    )?;

    window.set_position(
        (1920 - ORB_SIZE - 32) as isize,
        (1080 - ORB_SIZE - 32) as isize,
    );

    let mut buffer = vec![0u32; ORB_SIZE * ORB_SIZE];
    let mut state = OrbState::Idle;
    let start_time = std::time::Instant::now();

    while window.is_open() && !shutdown.load(Ordering::Relaxed) {
        while let Ok(s) = state_rx.try_recv() {
            state = s;
        }

        let time = start_time.elapsed().as_secs_f32();
        render_orb(&mut buffer, ORB_SIZE, ORB_SIZE, state, time);
        window.update_with_buffer(&buffer, ORB_SIZE, ORB_SIZE)?;

        std::thread::sleep(std::time::Duration::from_millis(FRAME_TIME_MS));
    }

    Ok(())
}

fn render_orb(buffer: &mut [u32], width: usize, height: usize, state: OrbState, time: f32) {
    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let max_radius = (width.min(height) as f32) * 0.38;
    let glow_radius = max_radius * 1.6;

    let (base_r, base_g, base_b) = state_color(state);

    let (pulse_speed, pulse_amp, glow_intensity) = match state {
        OrbState::Idle => (1.0, 0.15, 0.3),
        OrbState::Listening => (3.0, 0.25, 0.5),
        OrbState::Thinking => (5.0, 0.3, 0.6),
        OrbState::Speaking => (2.5, 0.2, 0.5),
        OrbState::Working => (4.0, 0.25, 0.55),
        OrbState::Error => (8.0, 0.35, 0.7),
        OrbState::Asleep => (0.5, 0.1, 0.15),
    };

    let pulse = 1.0 + pulse_amp * (time * pulse_speed).sin();
    let current_radius = max_radius * pulse;

    for y in 0..height {
        let dy = y as f32 - cy;
        for x in 0..width {
            let dx = x as f32 - cx;
            let dist = (dx * dx + dy * dy).sqrt();

            let (mut r, mut g, mut b, mut a) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);

            if dist < current_radius {
                let t = dist / current_radius;
                let brightness = 1.0 - t * 0.4;
                let edge = smoothstep(current_radius, current_radius - 2.0, dist);
                a = edge * brightness;

                let highlight = ((-dx * 0.3 + dy * 0.3) / current_radius + 0.5).clamp(0.0, 1.0);
                let specular = highlight.powf(3.0) * 0.4;
                r = (base_r + specular).min(1.0);
                g = (base_g + specular).min(1.0);
                b = (base_b + specular).min(1.0);
            } else if dist < glow_radius {
                let t = (dist - current_radius) / (glow_radius - current_radius);
                let glow = (1.0 - t).powf(2.0) * glow_intensity * pulse;
                a = glow * 0.3;
                r = base_r;
                g = base_g;
                b = base_b;
            }

            let ia = (a * 255.0) as u8;
            let ir = (r * a * 255.0) as u8;
            let ig = (g * a * 255.0) as u8;
            let ib = (b * a * 255.0) as u8;

            let idx = y * width + x;
            buffer[idx] =
                ((ia as u32) << 24) | ((ir as u32) << 16) | ((ig as u32) << 8) | (ib as u32);
        }
    }
}

fn state_color(state: OrbState) -> (f32, f32, f32) {
    match state {
        OrbState::Idle => (0.39, 0.71, 1.0),
        OrbState::Listening => (0.31, 0.86, 0.63),
        OrbState::Thinking => (1.0, 0.71, 0.31),
        OrbState::Speaking => (0.47, 1.0, 0.55),
        OrbState::Working => (0.71, 0.47, 1.0),
        OrbState::Error => (1.0, 0.31, 0.31),
        OrbState::Asleep => (0.24, 0.24, 0.31),
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
