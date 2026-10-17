//! NeoAmp's icon on its windows. Windows shows no icon on a window unless the program gives
//! it one, so there the icon that build.rs builds into neoamp.exe goes onto each window, for
//! the taskbar, Alt+Tab and the dialogs' title bars. Elsewhere the desktop finds the icon by
//! itself: from the desktop file on Linux, from the app bundle on macOS.

use winit::event_loop::ActiveEventLoop;
use winit::window::WindowAttributes;

/// The icon's resource number in neoamp.exe, which build.rs gives it.
#[cfg(target_os = "windows")]
const RESOURCE: u16 = 1;

/// `attrs` with NeoAmp's icon. It goes on with the window's other attributes, so the window
/// has it before it is shown: set afterwards, the taskbar button kept Windows' generic icon.
#[cfg(target_os = "windows")]
pub(crate) fn with_icon(attrs: WindowAttributes, el: &ActiveEventLoop) -> WindowAttributes {
    use winit::dpi::PhysicalSize;
    use winit::platform::windows::{IconExtWindows, WindowAttributesExtWindows};
    use winit::window::Icon;

    // The window's own scale is only known once it exists: take the main screen's.
    let scale = el.primary_monitor().map_or(1.0, |m| m.scale_factor());
    // Picked from the icon's sizes for that scale: the small icon is 16 pixels at 100%, the
    // large one (Alt+Tab, the taskbar) 32.
    let icon = |px: f64| {
        let px = (px * scale).round() as u32;
        Icon::from_resource(RESOURCE, Some(PhysicalSize::new(px, px)))
            .inspect_err(|e| log::debug!("no icon for the window: {e}"))
            .ok()
    };
    attrs
        .with_window_icon(icon(16.0))
        .with_taskbar_icon(icon(32.0))
}

/// Elsewhere the desktop finds the icon itself.
#[cfg(not(target_os = "windows"))]
pub(crate) fn with_icon(attrs: WindowAttributes, _el: &ActiveEventLoop) -> WindowAttributes {
    attrs
}
