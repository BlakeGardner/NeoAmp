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
const BACKSPACE: char = '⌫';
const RETURN: char = '↩';

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
            Style::Mac => &[
                OPTION,
                SHIFT,
                COMMAND,
                LEFT,
                RIGHT,
                FORWARD_DELETE,
                BACKSPACE,
                RETURN,
            ],
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
    Space,
    Backspace,
    /// Enter, Return on a Mac.
    Enter,
    Escape,
    ScrollLock,
    /// A function key: F1 to F24.
    F(u8),
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
                    Key::Space => f.write_str("Space"),
                    Key::Backspace => f.write_str("Backspace"),
                    Key::Enter => f.write_str("Enter"),
                    Key::Escape => f.write_str("Esc"),
                    Key::ScrollLock => f.write_str("Scroll Lock"),
                    Key::F(n) => write!(f, "F{n}"),
                }
            }
            Style::Mac => {
                // Apple's order is Control, Option, Shift, Command; Ctrl is not used here.
                for (held, symbol) in [(alt, OPTION), (shift, SHIFT), (ctrl, COMMAND)] {
                    if held {
                        f.write_char(symbol)?;
                    }
                }
                match key {
                    Key::Char(ch) => f.write_char(ch),
                    Key::Left => f.write_char(LEFT),
                    Key::Right => f.write_char(RIGHT),
                    Key::Delete => f.write_char(FORWARD_DELETE),
                    Key::Space => f.write_str("Space"),
                    Key::Backspace => f.write_char(BACKSPACE),
                    Key::Enter => f.write_char(RETURN),
                    // The built-in symbol font has no ⎋.
                    Key::Escape => f.write_str("Esc"),
                    Key::ScrollLock => f.write_str("Scroll Lock"),
                    Key::F(n) => write!(f, "F{n}"),
                }
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
        assert_eq!(mac(Shortcut::new(Key::Enter).alt()), "⌥↩");
        assert_eq!(mac(Shortcut::new(Key::Backspace)), "⌫");
        assert_eq!(mac(Shortcut::new(Key::Space)), "Space");
        assert_eq!(mac(Shortcut::char('K').ctrl().shift()), "⇧⌘K");
    }

    #[test]
    fn named_keys_are_spelt_out_the_windows_way() {
        assert_eq!(windows(Shortcut::new(Key::Enter).alt()), "Alt+Enter");
        assert_eq!(windows(Shortcut::new(Key::Space)), "Space");
        assert_eq!(windows(Shortcut::new(Key::Backspace)), "Backspace");
        assert_eq!(windows(Shortcut::new(Key::Escape)), "Esc");
        assert_eq!(windows(Shortcut::new(Key::ScrollLock)), "Scroll Lock");
        assert_eq!(windows(Shortcut::new(Key::F(14))), "F14");
    }

    #[test]
    fn the_symbols_cover_everything_a_style_writes_beyond_ascii() {
        let keys = [
            Key::Char('A'),
            Key::Left,
            Key::Right,
            Key::Delete,
            Key::Space,
            Key::Backspace,
            Key::Enter,
            Key::Escape,
            Key::ScrollLock,
            Key::F(14),
        ];
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
