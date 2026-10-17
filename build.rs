//! Builds NeoAmp's icon and version details into neoamp.exe on Windows, where Explorer, the
//! taskbar, the Start menu and Apps & features read them from the program itself. The icon,
//! `resources/icons/neoamp.ico`, comes from `windows/make-icon.py`. Nothing happens on other
//! platforms, whose desktops find the icon elsewhere (the Flatpak's desktop file, the macOS
//! app's Info.plist).

use std::env;

const ICON: &str = "resources/icons/neoamp.ico";

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed={ICON}");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    // Resource 1, which `window_icon` loads too. Windows names the program by its
    // FileDescription in Task Manager, the "now playing" overlay and the like.
    resource
        .set_icon(ICON)
        .set("FileDescription", "NeoAmp")
        .set("ProductName", "NeoAmp")
        .set("OriginalFilename", "neoamp.exe")
        .set("InternalName", "neoamp");
    // The binary only: the tests do without.
    if let Err(e) = resource.compile_for("neoamp") {
        panic!(
            "cannot build the icon into neoamp.exe: {e}. It needs the resource compiler rc.exe \
             from the Windows SDK, which Rust's MSVC toolchain needs anyway (or windres with \
             the GNU toolchain)."
        );
    }
}
