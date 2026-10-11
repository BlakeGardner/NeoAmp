//! A pixel-exact Winamp 2.x classic interface, drawn from real .wsz skins.

mod canvas;
mod eq_window;
mod font;
mod main_window;
mod playlist_window;
mod skin;
mod sprites;
mod ui;
mod vis;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use pixels::wgpu;
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use canvas::Image;
use skin::Skin;
use ui::{Action, KeyCmd, Ui};

/// The stock Winamp 2.91 skin, embedded as the fallback for every missing sheet.
const BASE_SKIN: &[u8] = include_bytes!("../skins/base-2.91.wsz");
const FRAME: Duration = Duration::from_millis(33);

const USAGE: &str = "\
usage: winamp [--skin <file.wsz>] [--screenshot <out.png>] [<file.wsz>]

  <file.wsz>           load a classic Winamp 2.x skin (also: drop one onto the window)
  --screenshot <png>   render one frame to a PNG and exit

keys: Z/X/C/V/B transport, S shuffle, R repeat, Ctrl+T time mode,
      Ctrl+D double size, Alt+G equalizer, Alt+E playlist, arrows volume/seek";

struct App {
    ui: Ui,
    skin: Skin,
    base: Skin,
    font: Option<font::TtfFont>,
    canvas: Image,
    window: Option<Arc<Window>>,
    pixels: Option<Pixels<'static>>,
    cursor: PhysicalPosition<f64>,
    modifiers: ModifiersState,
    next_frame: Instant,
}

impl App {
    fn load_skin(&mut self, path: &Path) {
        match load_skin(path, &self.base) {
            Ok(s) => {
                log::info!("loaded skin {}", s.name);
                self.skin = s;
            }
            Err(e) => log::error!("cannot load skin {}: {e:#}", path.display()),
        }
    }

    fn canvas_pos(&self) -> (i32, i32) {
        let Some(p) = &self.pixels else { return (-1, -1) };
        let pos = (self.cursor.x as f32, self.cursor.y as f32);
        let (x, y) = match p.window_pos_to_pixel(pos) {
            Ok(xy) => xy,
            Err(xy) => p.clamp_pixel_pos(xy),
        };
        (x as i32, y as i32)
    }

    fn relayout(&mut self) {
        let (w, h) = self.ui.layout().size;
        let scale = self.ui.scale() as i32;
        self.canvas = Image::new(w, h);
        if let Some(p) = &mut self.pixels
            && let Err(e) = p.resize_buffer(w as u32, h as u32)
        {
            log::error!("resize_buffer: {e}");
        }
        if let Some(win) = &self.window {
            let _ = win.request_inner_size(LogicalSize::new((w * scale) as f64, (h * scale) as f64));
            win.request_redraw();
        }
    }

    fn handle(&mut self, actions: Vec<Action>, el: &ActiveEventLoop) {
        for a in actions {
            match a {
                Action::DragWindow => {
                    if let Some(w) = &self.window
                        && let Err(e) = w.drag_window()
                    {
                        log::warn!("drag_window: {e}");
                    }
                }
                Action::Close => el.exit(),
                Action::Minimize => {
                    if let Some(w) = &self.window {
                        w.set_minimized(true);
                    }
                }
                Action::Relayout => self.relayout(),
            }
        }
    }

    fn key_cmd(&self, key: &Key) -> Option<KeyCmd> {
        let ctrl = self.modifiers.control_key();
        let alt = self.modifiers.alt_key();
        Some(match key {
            Key::Character(s) => match (s.to_ascii_lowercase().as_str(), ctrl, alt) {
                ("d", true, _) => KeyCmd::ToggleDoubleSize,
                ("t", true, _) => KeyCmd::ToggleTimeMode,
                ("g", _, true) => KeyCmd::ToggleEq,
                ("e", _, true) => KeyCmd::TogglePl,
                ("z", false, false) => KeyCmd::Prev,
                ("x", false, false) => KeyCmd::Play,
                ("c", false, false) => KeyCmd::Pause,
                ("v", false, false) => KeyCmd::Stop,
                ("b", false, false) => KeyCmd::Next,
                ("r", false, false) => KeyCmd::ToggleRepeat,
                ("s", false, false) => KeyCmd::ToggleShuffle,
                _ => return None,
            },
            Key::Named(NamedKey::ArrowUp) => KeyCmd::VolumeUp,
            Key::Named(NamedKey::ArrowDown) => KeyCmd::VolumeDown,
            Key::Named(NamedKey::ArrowLeft) => KeyCmd::SeekBack,
            Key::Named(NamedKey::ArrowRight) => KeyCmd::SeekForward,
            _ => return None,
        })
    }

    fn render(&mut self) {
        let now = Instant::now();
        let (w, h) = self.ui.layout().size;
        if self.canvas.w != w || self.canvas.h != h {
            self.canvas = Image::new(w, h);
        }
        self.ui.draw(&mut self.canvas, &self.skin, self.font.as_ref(), now);
        if let Some(p) = &mut self.pixels {
            let frame = p.frame_mut();
            if frame.len() == self.canvas.data.len() {
                frame.copy_from_slice(&self.canvas.data);
            }
            if let Err(e) = p.render() {
                log::error!("render: {e}");
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (w, h) = self.ui.layout().size;
        let scale = self.ui.scale() as i32;
        let attrs = Window::default_attributes()
            .with_title("Winamp")
            .with_inner_size(LogicalSize::new((w * scale) as f64, (h * scale) as f64))
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true);
        let window = Arc::new(el.create_window(attrs).expect("cannot create window"));
        let size = window.inner_size();
        let make_surface = || SurfaceTexture::new(size.width.max(1), size.height.max(1), window.clone());
        let pixels = PixelsBuilder::new(w as u32, h as u32, make_surface())
            .clear_color(wgpu::Color::TRANSPARENT)
            .alpha_mode(wgpu::CompositeAlphaMode::PreMultiplied)
            .build()
            .or_else(|e| {
                log::warn!("transparent surface unavailable ({e}); falling back to opaque");
                PixelsBuilder::new(w as u32, h as u32, make_surface())
                    .clear_color(wgpu::Color::BLACK)
                    .build()
            })
            .expect("cannot create pixel buffer");
        self.canvas = Image::new(w, h);
        self.pixels = Some(pixels);
        self.window = Some(window);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let now = Instant::now();
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(p) = &mut self.pixels
                    && let Err(e) = p.resize_surface(size.width.max(1), size.height.max(1))
                {
                    log::error!("resize_surface: {e}");
                }
            }
            WindowEvent::Focused(f) => self.ui.active = f,
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                let (x, y) = self.canvas_pos();
                let actions = self.ui.mouse_move(x, y, now);
                self.handle(actions, el);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let (x, y) = self.canvas_pos();
                let actions = match state {
                    ElementState::Pressed => self.ui.mouse_down(x, y, now),
                    ElementState::Released => self.ui.mouse_up(x, y, now),
                };
                self.handle(actions, el);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && let Some(cmd) = self.key_cmd(&event.logical_key)
                {
                    let actions = self.ui.key(cmd, now);
                    self.handle(actions, el);
                }
            }
            WindowEvent::DroppedFile(path) => self.load_skin(&path),
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_frame {
            self.next_frame = now + FRAME;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

fn load_skin(path: &Path, base: &Skin) -> Result<Skin> {
    let mut skin = Skin::from_path(path)?;
    skin.fill_missing_from(base);
    Ok(skin)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,sctk_adwaita=error")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut skin_path: Option<PathBuf> = None;
    let mut screenshot: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" => {
                screenshot = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--skin" => {
                skin_path = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => skin_path = Some(PathBuf::from(other)),
        }
        i += 1;
    }

    let base = Skin::from_wsz(BASE_SKIN, "base-2.91").context("embedded base skin")?;
    let skin = match &skin_path {
        Some(p) => load_skin(p, &base)?,
        None => base.clone(),
    };
    let font = font::TtfFont::load_system(font::TtfFont::DEFAULT_PX);
    let mut ui = Ui::new();

    if let Some(out) = screenshot {
        // Advance the fake playback a little so the frame looks alive.
        let start = Instant::now();
        let (w, h) = ui.layout().size;
        let mut canvas = Image::new(w, h);
        for k in 0..70 {
            ui.draw(&mut canvas, &skin, font.as_ref(), start + FRAME * k);
        }
        image::save_buffer(&out, &canvas.data, w as u32, h as u32, image::ColorType::Rgba8)
            .with_context(|| format!("writing {}", out.display()))?;
        println!("wrote {} ({w}x{h})", out.display());
        return Ok(());
    }

    let event_loop = EventLoop::new()?;
    let mut app = App {
        ui,
        skin,
        base,
        font,
        canvas: Image::new(1, 1),
        window: None,
        pixels: None,
        cursor: PhysicalPosition::new(0.0, 0.0),
        modifiers: ModifiersState::empty(),
        next_frame: Instant::now(),
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
