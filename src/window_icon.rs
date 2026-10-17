//! NeoAmp's icon on its windows. Windows shows no icon on a window unless the program gives
//! it one, so there the icon that build.rs builds into neoamp.exe goes onto each window, for
//! the taskbar, Alt+Tab and the dialogs' title bars. Elsewhere the desktop finds the icon by
//! itself: from the desktop file on Linux, from the app bundle on macOS.

use winit::window::Window;

/// The icon's resource number in neoamp.exe, which build.rs gives it.
#[cfg(target_os = "windows")]
const RESOURCE: u16 = 1;

/// Give `window` NeoAmp's icon, where the platform needs telling.
pub(crate) fn set(window: &Window) {
    #[cfg(target_os = "windows")]
    {
        use winit::dpi::PhysicalSize;
        use winit::platform::windows::{IconExtWindows, WindowExtWindows};
        use winit::window::Icon;

        let scale = window.scale_factor();
        // Picked from the icon's sizes for the desktop's scale: the small icon is 16 pixels at
        // 100%, the large one (Alt+Tab, the taskbar) 32.
        let icon = |px: f64| {
            let px = (px * scale).round() as u32;
            Icon::from_resource(RESOURCE, Some(PhysicalSize::new(px, px)))
                .inspect_err(|e| log::debug!("no icon for the window: {e}"))
                .ok()
        };
        window.set_window_icon(icon(16.0));
        window.set_taskbar_icon(icon(32.0));
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}
