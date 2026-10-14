//! Interface zoom. Classic skins were drawn for one screen pixel per skin pixel on the
//! 800x600 and 1024x768 screens of their day, which looks tiny on today's displays.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Screen height, in logical pixels, that suits a 1x skin. Taller screens get proportionally
/// more zoom, rounded down to a half step: 1080 lines stay at 1x, 1440 get 1.5x, 2160 get 2x.
const REFERENCE_HEIGHT: f64 = 960.0;

/// How far a factor may sit from a half step and still count as one (absorbs float noise
/// such as `150% / 100`).
const STEP_TOLERANCE: f64 = 1e-9;

/// How much the skin is enlarged, in half steps from 1x to 3x. Double size multiplies this
/// by two, and the desktop's own scale factor applies on top of both.
///
/// Saved in the settings as its factor (`1.5`); reading anything between the steps fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Zoom {
    halves: u8,
}

impl Zoom {
    /// 1x: one screen pixel per skin pixel, as the skins were drawn.
    pub const MIN: Zoom = Zoom { halves: 2 };
    /// 3x, the largest step.
    pub const MAX: Zoom = Zoom { halves: 6 };

    /// The default zoom for a screen `height` logical pixels tall.
    #[must_use]
    pub fn for_screen_height(height: f64) -> Zoom {
        let halves = (height / REFERENCE_HEIGHT * 2.0).floor();
        if halves.is_nan() {
            return Self::MIN;
        }
        Self::from_whole_halves(halves.clamp(Self::MIN.halves_f64(), Self::MAX.halves_f64()))
    }

    /// Screen pixels per skin pixel, before double size and the desktop's scale factor.
    #[must_use]
    pub fn factor(self) -> f64 {
        self.halves_f64() / 2.0
    }

    /// One half step larger, stopping at [`Zoom::MAX`].
    #[must_use]
    pub const fn larger(self) -> Zoom {
        let halves = self.halves.saturating_add(1);
        if halves > Self::MAX.halves {
            Self::MAX
        } else {
            Zoom { halves }
        }
    }

    /// One half step smaller, stopping at [`Zoom::MIN`].
    #[must_use]
    pub const fn smaller(self) -> Zoom {
        let halves = self.halves.saturating_sub(1);
        if halves < Self::MIN.halves {
            Self::MIN
        } else {
            Zoom { halves }
        }
    }

    fn halves_f64(self) -> f64 {
        f64::from(self.halves)
    }

    /// `halves` must already be a whole number of half steps between [`Zoom::MIN`] and
    /// [`Zoom::MAX`], which makes the cast exact.
    const fn from_whole_halves(halves: f64) -> Zoom {
        Zoom {
            halves: halves as u8,
        }
    }
}

impl Default for Zoom {
    fn default() -> Self {
        Self::MIN
    }
}

impl From<Zoom> for f64 {
    fn from(zoom: Zoom) -> f64 {
        zoom.factor()
    }
}

impl fmt::Display for Zoom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x", self.factor())
    }
}

/// The zoom that enlarges the skin `factor` times; only half steps from 1 to 3 exist.
impl TryFrom<f64> for Zoom {
    type Error = ZoomError;

    fn try_from(factor: f64) -> Result<Self, Self::Error> {
        let halves = factor * 2.0;
        let step = halves.round();
        let in_range = (Self::MIN.halves_f64()..=Self::MAX.halves_f64()).contains(&step);
        if in_range && (halves - step).abs() < STEP_TOLERANCE {
            Ok(Self::from_whole_halves(step))
        } else {
            Err(ZoomError(factor.to_string()))
        }
    }
}

/// Accepts `1.5`, `1.5x` or `150%`.
impl FromStr for Zoom {
    type Err = ZoomError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let factor = match t.strip_suffix('%') {
            Some(percent) => percent.trim().parse::<f64>().map(|p| p / 100.0),
            None => t
                .strip_suffix(['x', 'X'])
                .unwrap_or(t)
                .trim()
                .parse::<f64>(),
        };
        // Either failure is reported with the text as given, which says more than the
        // float parser's own message would.
        let invalid = || ZoomError(t.to_owned());
        factor
            .map_err(|_| invalid())
            .and_then(|f| Zoom::try_from(f).map_err(|_| invalid()))
    }
}

/// A scale that is not one of the zoom steps (1, 1.5, 2, 2.5 or 3). Holds what was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoomError(String);

impl fmt::Display for ZoomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the scale must be 1, 1.5, 2, 2.5 or 3, not {:?}", self.0)
    }
}

impl std::error::Error for ZoomError {}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn zoom(factor: f64) -> Zoom {
        Zoom::try_from(factor).unwrap()
    }

    #[test]
    fn only_half_steps_from_one_to_three_exist() {
        assert_eq!(zoom(1.0), Zoom::MIN);
        assert_eq!(zoom(3.0), Zoom::MAX);
        assert!((zoom(2.5).factor() - 2.5).abs() < STEP_TOLERANCE);
        for bad in [0.5, 1.25, 3.5, -1.0, f64::NAN, f64::INFINITY] {
            assert!(Zoom::try_from(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn stepping_stops_at_the_ends() {
        assert_eq!(zoom(1.5).larger(), zoom(2.0));
        assert_eq!(zoom(1.5).smaller(), zoom(1.0));
        assert_eq!(Zoom::MAX.larger(), Zoom::MAX);
        assert_eq!(Zoom::MIN.smaller(), Zoom::MIN);
    }

    #[test]
    fn default_follows_the_screen_height() {
        for (height, factor) in [
            (600.0, 1.0),
            (1080.0, 1.0),
            (1200.0, 1.0),
            (1440.0, 1.5),
            (1600.0, 1.5),
            (2160.0, 2.0),
            (2880.0, 3.0),
            (4320.0, 3.0),
            (f64::NAN, 1.0),
        ] {
            assert_eq!(Zoom::for_screen_height(height), zoom(factor), "{height}");
        }
    }

    #[test]
    fn parses_and_prints_factors() {
        assert_eq!("1.5".parse::<Zoom>(), Ok(zoom(1.5)));
        assert_eq!(" 2x ".parse::<Zoom>(), Ok(zoom(2.0)));
        assert_eq!("250%".parse::<Zoom>(), Ok(zoom(2.5)));
        assert_eq!(zoom(1.0).to_string(), "1x");
        assert_eq!(zoom(1.5).to_string(), "1.5x");
    }

    #[test]
    fn rejected_input_is_named_in_the_error() {
        assert_eq!("1.3".parse::<Zoom>(), Err(ZoomError("1.3".into())));
        assert_eq!(" big ".parse::<Zoom>(), Err(ZoomError("big".into())));
        assert_eq!(
            ZoomError("big".into()).to_string(),
            r#"the scale must be 1, 1.5, 2, 2.5 or 3, not "big""#
        );
    }
}
