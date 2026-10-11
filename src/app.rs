//! The desktop application: a borderless, transparent winit window presenting the
//! software framebuffer through `pixels`, plus the offscreen screenshot mode.

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

use crate::canvas::Image;
use crate::font::TtfFont;
use crate::skin::Skin;
use crate::ui::{Action, KeyCmd, Ui};

/// The stock Winamp 2.91 skin, embedded as the fallback for every missing sheet.
pub const BASE_SKIN: &[u8] = include_bytes!("../skins/base-2.91.wsz");
const FRAME: Duration = Duration::from_millis(33);

/// What to run: an optional skin to load and an optional screenshot-and-exit path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub skin: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
}

/// Load the base skin, apply the options and either render a screenshot or open the window.
///
/// # Errors
/// Fails if the embedded base skin or the requested skin cannot be parsed, if the screenshot
/// cannot be written, or if the windowing system cannot be initialised.
pub fn run(opts: Options) -> Result<()> {
    let base = Skin::from_wsz(BASE_SKIN, "base-2.91").context("embedded base skin")?;
    let custom = opts
        .skin
        .as_deref()
        .map(|p| load_skin(p, &base))
        .transpose()?;
    let font = TtfFont::load_system(TtfFont::DEFAULT_PX);
    let ui = Ui::new();

    if let Some(out) = &opts.screenshot {
        return render_screenshot(ui, custom.as_ref().unwrap_or(&base), font.as_ref(), out);
    }

    let event_loop = EventLoop::new().context("creating the event loop")?;
    let mut app = App {
        ui,
        base,
        custom,
        font,
        canvas: Image::new(1, 1),
        window: None,
        pixels: None,
        cursor: PhysicalPosition::new(0.0, 0.0),
        modifiers: ModifiersState::empty(),
        next_frame: Instant::now(),
    };
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;
    Ok(())
}

/// Draw a few frames of fake playback offscreen and save the last one as a PNG.
///
/// # Errors
/// Fails if the PNG cannot be written.
pub fn render_screenshot(
    mut ui: Ui,
    skin: &Skin,
    font: Option<&TtfFont>,
    out: &Path,
) -> Result<()> {
    let start = Instant::now();
    let (w, h) = ui.layout().size;
    let mut canvas = Image::new(w, h);
    for k in 0..70 {
        ui.draw(&mut canvas, skin, font, start + FRAME * k);
    }
    let (pw, ph) = (u32::try_from(w)?, u32::try_from(h)?);
    image::save_buffer(out, &canvas.data, pw, ph, image::ColorType::Rgba8)
        .with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {} ({w}x{h})", out.display());
    Ok(())
}

fn load_skin(path: &Path, base: &Skin) -> Result<Skin> {
    let mut skin = Skin::from_path(path)?;
    skin.fill_missing_from(base);
    Ok(skin)
}

struct App {
    ui: Ui,
    base: Skin,
    /// A user-chosen skin; `None` means the embedded base skin is shown.
    custom: Option<Skin>,
    font: Option<TtfFont>,
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
                self.custom = Some(s);
            }
            Err(e) => log::error!("cannot load skin {}: {e:#}", path.display()),
        }
    }

    fn create_window(&mut self, el: &ActiveEventLoop) -> Result<()> {
        let (w, h) = self.ui.layout().size;
        let scale = i32::from(self.ui.scale());
        let attrs = Window::default_attributes()
            .with_title("Winamp")
            .with_inner_size(LogicalSize::new(f64::from(w * scale), f64::from(h * scale)))
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true);
        let window = Arc::new(el.create_window(attrs).context("creating the window")?);
        let size = window.inner_size();
        let (bw, bh) = (u32::try_from(w)?, u32::try_from(h)?);
        let make_surface =
            || SurfaceTexture::new(size.width.max(1), size.height.max(1), window.clone());
        let pixels = PixelsBuilder::new(bw, bh, make_surface())
            .clear_color(wgpu::Color::TRANSPARENT)
            .alpha_mode(wgpu::CompositeAlphaMode::PreMultiplied)
            .build()
            .or_else(|e| {
                log::warn!("transparent surface unavailable ({e}); falling back to opaque");
                PixelsBuilder::new(bw, bh, make_surface())
                    .clear_color(wgpu::Color::BLACK)
                    .build()
            })
            .context("creating the GPU surface")?;
        self.canvas = Image::new(w, h);
        self.pixels = Some(pixels);
        self.window = Some(window);
        Ok(())
    }

    fn canvas_pos(&self) -> (i32, i32) {
        let Some(p) = &self.pixels else {
            return (-1, -1);
        };
        let pos = (self.cursor.x as f32, self.cursor.y as f32);
        let (x, y) = match p.window_pos_to_pixel(pos) {
            Ok(xy) => xy,
            Err(xy) => p.clamp_pixel_pos(xy),
        };
        (
            i32::try_from(x).unwrap_or(i32::MAX),
            i32::try_from(y).unwrap_or(i32::MAX),
        )
    }

    fn relayout(&mut self) {
        let (w, h) = self.ui.layout().size;
        let scale = i32::from(self.ui.scale());
        self.canvas = Image::new(w, h);
        if let (Some(p), Ok(bw), Ok(bh)) = (&mut self.pixels, u32::try_from(w), u32::try_from(h))
            && let Err(e) = p.resize_buffer(bw, bh)
        {
            log::error!("resize_buffer: {e}");
        }
        if let Some(win) = &self.window {
            let _ = win
                .request_inner_size(LogicalSize::new(f64::from(w * scale), f64::from(h * scale)));
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
        let skin = self.custom.as_ref().unwrap_or(&self.base);
        self.ui
            .draw(&mut self.canvas, skin, self.font.as_ref(), now);
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
        if let Err(e) = self.create_window(el) {
            log::error!("cannot open a window: {e:#}");
            el.exit();
        }
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
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
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
