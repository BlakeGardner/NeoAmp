//! Opening a web page in the system's browser, each platform's own way: `NSWorkspace` on macOS,
//! the shell's `ShellExecuteEx` on Windows (through the [open](https://crates.io/crates/open)
//! crate), and the desktop portal's `OpenURI` over D-Bus on Linux, which works inside the
//! Flatpak too. No command line is run.

use anyhow::Result;

/// Open `url`, an `https://` address, in the system's browser, on a thread of its own so that
/// a slow desktop never holds up the interface. Failures are logged.
pub fn open(url: &str) {
    if !is_web_address(url) {
        log::warn!("not opening {url:?}: only https addresses are opened");
        return;
    }
    let url = url.to_owned();
    let spawned =
        std::thread::Builder::new()
            .name("open link".into())
            .spawn(move || match open_now(&url) {
                Ok(()) => log::info!("opened {url} in the browser"),
                Err(e) => log::warn!("cannot open {url} in the browser: {e:#}"),
            });
    if let Err(e) = spawned {
        log::warn!("cannot open the link: {e}");
    }
}

/// Whether `url` is an `https://` address with nothing in it a browser could take for more.
fn is_web_address(url: &str) -> bool {
    url.get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        && url.len() > 8
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[cfg(target_os = "macos")]
fn open_now(url: &str) -> Result<()> {
    use anyhow::{Context as _, bail};
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    let url =
        NSURL::URLWithString(&NSString::from_str(url)).context("not an address macOS takes")?;
    if !NSWorkspace::sharedWorkspace().openURL(&url) {
        bail!("macOS would not open it");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_now(url: &str) -> Result<()> {
    open::that_detached(url)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_now(url: &str) -> Result<()> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    let connection = zbus::blocking::Connection::session()?;
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    // No parent window: the portal shows its own dialog, if any, on its own.
    connection.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.OpenURI"),
        "OpenURI",
        &("", url, options),
    )?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn open_now(_url: &str) -> Result<()> {
    anyhow::bail!("NeoAmp knows no way to open links on this system")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_addresses_are_opened() {
        assert!(is_web_address("https://skins.webamp.org/"));
        assert!(is_web_address("HTTPS://skins.webamp.org"));
        for not in [
            "http://skins.webamp.org/",
            "file:///etc/passwd",
            "https://",
            "https://a b",
            "https://a\nb",
            "skins.webamp.org",
            "",
        ] {
            assert!(!is_web_address(not), "{not:?}");
        }
    }
}
