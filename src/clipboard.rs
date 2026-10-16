//! The system clipboard's text, for pasting into the dialogs' boxes, through
//! [arboard](https://crates.io/crates/arboard): the native clipboard on Windows and macOS, and
//! X11's on Linux, which XWayland shares with Wayland applications. Under pure Wayland, with
//! no XWayland, there is none (logged).

/// The text on the clipboard; `None` when it holds no text or cannot be read (logged).
pub(crate) fn text() -> Option<String> {
    let read = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text());
    match read {
        Ok(text) => Some(text),
        Err(arboard::Error::ContentNotAvailable) => None,
        Err(e) => {
            log::warn!("cannot read the clipboard: {e}");
            None
        }
    }
}

/// The clipboard's text if it is one web address, `http://` or `https://`, as the Open URL
/// dialog starts with.
pub(crate) fn web_url() -> Option<String> {
    let text = text()?;
    let text = text.trim();
    (crate::audio::is_web_url(text) && !text.contains(char::is_whitespace)).then(|| text.to_owned())
}
