//! Keyboard shortcuts as the menus write them. Winamp's are written the Windows way
//! ("Ctrl+Shift+1"). On macOS Cmd takes Ctrl's place, as in Mac apps, and shortcuts are
//! written with the Mac's key symbols ("⇧⌘1").

use std::fmt::{self, Write as _};

const OPTION: char = '⌥';
const SHIFT: char = '⇧';
const COMMAND: char = '⌘';
const LEFT: char = '←';
const RIGHT: char = '→';
const FORWARD_DELETE: char = '⌦';

/// How shortcuts are written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// "Ctrl+Shift+1", on Windows and Linux.
    Windows,
    /// "⇧⌘1": the modifiers' symbols in Apple's order, with nothing between them.
    Mac,
}

impl Style {
    /// This platform's style.
    pub const CURRENT: Style = if cfg!(target_os = "macos") {
        Style::Mac
    } else {
        Style::Windows
    };

    /// The symbols this style writes, which a menu font may not have.
    #[must_use]
    pub const fn symbols(self) -> &'static [char] {
        match self {
            Style::Windows => &[],
            Style::Mac => &[OPTION, SHIFT, COMMAND, LEFT, RIGHT, FORWARD_DELETE],
        }
    }
}

/// The key pressed with the modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// A key named by the character it types: `'A'`, `'1'`, `'='`.
    Char(char),
    Left,
    Right,
    /// Delete, which is forward delete on a Mac.
    Delete,
}

/// A key and the modifiers held with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shortcut {
    ctrl: bool,
    alt: bool,
    shift: bool,
    key: Key,
}

impl Shortcut {
    /// `key` on its own.
    #[must_use]
    pub const fn new(key: Key) -> Shortcut {
        Shortcut {
            ctrl: false,
            alt: false,
            shift: false,
            key,
        }
    }

    /// The key that types `ch`, on its own.
    #[must_use]
    pub const fn char(ch: char) -> Shortcut {
        Self::new(Key::Char(ch))
    }

    /// With Ctrl held, or Cmd on macOS.
    #[must_use]
    pub const fn ctrl(self) -> Shortcut {
        Shortcut { ctrl: true, ..self }
    }

    /// With Alt held (Option on macOS).
    #[must_use]
    pub const fn alt(self) -> Shortcut {
        Shortcut { alt: true, ..self }
    }

    /// With Shift held.
    #[must_use]
    pub const fn shift(self) -> Shortcut {
        Shortcut {
            shift: true,
            ..self
        }
    }

    /// This shortcut as `style` writes it.
    #[must_use]
    pub const fn written(self, style: Style) -> Written {
        Written {
            shortcut: self,
            style,
        }
    }
}

/// Written in this platform's style.
impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.written(Style::CURRENT).fmt(f)
    }
}

/// A shortcut as a given style writes it (see [`Shortcut::written`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Written {
    shortcut: Shortcut,
    style: Style,
}

impl fmt::Display for Written {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Shortcut {
            ctrl,
            alt,
            shift,
            key,
        } = self.shortcut;
        match self.style {
            Style::Windows => {
                for (held, name) in [(ctrl, "Ctrl+"), (alt, "Alt+"), (shift, "Shift+")] {
                    if held {
                        f.write_str(name)?;
                    }
                }
                match key {
                    Key::Char(ch) => f.write_char(ch),
                    Key::Left => f.write_str("Left"),
                    Key::Right => f.write_str("Right"),
                    Key::Delete => f.write_str("Del"),
                }
            }
            Style::Mac => {
                // Apple's order is Control, Option, Shift, Command; Ctrl is not used here.
                for (held, symbol) in [(alt, OPTION), (shift, SHIFT), (ctrl, COMMAND)] {
                    if held {
                        f.write_char(symbol)?;
                    }
                }
                f.write_char(match key {
                    Key::Char(ch) => ch,
                    Key::Left => LEFT,
                    Key::Right => RIGHT,
                    Key::Delete => FORWARD_DELETE,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(s: Shortcut) -> String {
        s.written(Style::Windows).to_string()
    }

    fn mac(s: Shortcut) -> String {
        s.written(Style::Mac).to_string()
    }

    #[test]
    fn the_windows_style_spells_the_modifiers_out() {
        assert_eq!(windows(Shortcut::char('1').shift().ctrl()), "Ctrl+Shift+1");
        assert_eq!(windows(Shortcut::char('E').alt()), "Alt+E");
        assert_eq!(windows(Shortcut::new(Key::Delete).alt()), "Alt+Del");
        assert_eq!(windows(Shortcut::char('L').shift()), "Shift+L");
        assert_eq!(windows(Shortcut::new(Key::Left)), "Left");
        assert_eq!(windows(Shortcut::char('=').ctrl()), "Ctrl+=");
    }

    #[test]
    fn the_mac_style_writes_cmd_for_ctrl_with_symbols_in_apples_order() {
        assert_eq!(mac(Shortcut::char('1').shift().ctrl()), "⇧⌘1");
        assert_eq!(mac(Shortcut::char('R').ctrl().shift()), "⇧⌘R");
        assert_eq!(mac(Shortcut::char('E').alt()), "⌥E");
        assert_eq!(mac(Shortcut::new(Key::Delete).alt()), "⌥⌦");
        assert_eq!(mac(Shortcut::char('L').shift()), "⇧L");
        assert_eq!(mac(Shortcut::new(Key::Right)), "→");
        assert_eq!(mac(Shortcut::char('=').ctrl()), "⌘=");
        assert_eq!(mac(Shortcut::char('Z')), "Z");
    }

    #[test]
    fn the_symbols_cover_everything_a_style_writes_beyond_ascii() {
        let keys = [Key::Char('A'), Key::Left, Key::Right, Key::Delete];
        for style in [Style::Windows, Style::Mac] {
            for key in keys {
                let all = Shortcut::new(key)
                    .ctrl()
                    .alt()
                    .shift()
                    .written(style)
                    .to_string();
                for ch in all.chars().filter(|ch| !ch.is_ascii()) {
                    assert!(style.symbols().contains(&ch), "{style:?} lacks {ch}");
                }
            }
        }
    }
}
