//! Which preset MilkDrop shows and when it moves on, by MilkDrop's own rules: a preset runs
//! for 16 seconds plus up to 10 more chosen at random, then blends into the next over 2.7
//! seconds; the user's changes blend over 1.7 seconds or cut at once. Presets come in random
//! order (the default) or in the order they are listed, and in random order Previous goes
//! back through the presets shown before, which Next then follows forward again.

use std::time::Instant;

use super::engine::{Engine, Frame};
use super::presets::Presets;
use crate::playlist::Rng;

/// Seconds a preset blends into the next when MilkDrop moves on by itself (MilkDrop's
/// `m_fBlendTimeAuto`).
pub const BLEND_AUTO: f64 = 2.7;
/// Seconds a preset blends into the next when the user asks for another (`m_fBlendTimeUser`).
pub const BLEND_USER: f64 = 1.7;
/// Seconds a preset runs before the next blends in, by default (`m_fTimeBetweenPresets`).
pub const DEFAULT_SECONDS: f64 = 16.0;
/// The random extra time, as a share of the time between presets (MilkDrop's
/// `m_fTimeBetweenPresetsRand` of 10 seconds, against 16).
const RANDOM_SHARE: f64 = 0.625;
/// The fewest and most seconds between presets that can be chosen.
pub const SECONDS_RANGE: (f64, f64) = (1.0, 3600.0);
/// How many presets Previous can go back through.
const HISTORY: usize = 64;

/// A change of preset the user asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// The next preset, blending into it (MilkDrop's Space).
    Next,
    /// The next preset at once (H).
    HardCut,
    /// The preset shown before (Backspace).
    Previous,
    /// Preset `n` of the list.
    Select(usize),
}

/// MilkDrop's presets and when they change.
#[derive(Debug)]
pub struct Visualizer {
    engine: Engine,
    presets: Presets,
    /// The preset showing, by its place in `presets`.
    current: Option<usize>,
    /// The preset to start with, by name, until one is showing.
    wanted: Option<String>,
    /// Presets shown, oldest first; `history[pos]` is the current one.
    history: Vec<usize>,
    pos: usize,
    random: bool,
    locked: bool,
    seconds: f64,
    /// When the next preset blends in by itself; `None` until worked out for the preset
    /// showing.
    next_change: Option<Instant>,
    rng: Rng,
}

impl Visualizer {
    /// Showing `presets`, in random order, with random numbers from `seed`.
    #[must_use]
    pub fn new(presets: Presets, seed: u64) -> Visualizer {
        Visualizer {
            engine: Engine::new(seed),
            presets,
            current: None,
            wanted: None,
            history: Vec::new(),
            pos: 0,
            random: true,
            locked: false,
            seconds: DEFAULT_SECONDS,
            next_change: None,
            rng: Rng::with_seed(seed),
        }
    }

    #[must_use]
    pub fn presets(&self) -> &Presets {
        &self.presets
    }

    /// List `presets` instead, keeping the one showing if it is among them. One that is not
    /// is replaced at the next [`Visualizer::tick`].
    pub fn set_presets(&mut self, presets: Presets) {
        let showing = self
            .current
            .and_then(|i| self.presets.name(i))
            .map(str::to_owned);
        self.presets = presets;
        self.current = showing.as_deref().and_then(|n| self.presets.find(n));
        if showing.is_some() && self.current.is_none() {
            self.wanted = showing;
        }
        self.history = self.current.into_iter().collect();
        self.pos = 0;
    }

    /// The preset showing, by its place in [`Visualizer::presets`].
    #[must_use]
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// The name of the preset showing, or else of the one it will start with.
    #[must_use]
    pub fn current_name(&self) -> Option<&str> {
        match self.current {
            Some(i) => self.presets.name(i),
            None => self.wanted.as_deref(),
        }
    }

    /// Start with the preset called `name`, when one is called that.
    pub fn start_with(&mut self, name: Option<String>) {
        if self.current.is_none() {
            self.wanted = name;
        }
    }

    /// Presets come in random order, rather than the order they are listed in.
    #[must_use]
    pub fn random(&self) -> bool {
        self.random
    }

    /// Choose the order; the presets shown so far are forgotten, as in MilkDrop.
    pub fn set_random(&mut self, random: bool) {
        self.random = random;
        self.history = self.current.into_iter().collect();
        self.pos = 0;
    }

    /// The preset stays until the user changes it.
    #[must_use]
    pub fn locked(&self) -> bool {
        self.locked
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.next_change = None;
    }

    /// How long each preset runs, before the random extra.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.seconds
    }

    /// Run each preset `seconds`, kept within [`SECONDS_RANGE`]; anything else is ignored.
    pub fn set_seconds(&mut self, seconds: f64) {
        if seconds.is_finite() {
            self.seconds = seconds.clamp(SECONDS_RANGE.0, SECONDS_RANGE.1);
            self.next_change = None;
        }
    }

    /// Change the preset as the user asked.
    pub fn change(&mut self, change: Change) {
        match change {
            Change::Next => self.next(BLEND_USER),
            Change::HardCut => self.next(0.0),
            Change::Previous => self.previous(),
            Change::Select(i) => {
                if i < self.presets.len() {
                    self.show(i, 0.0, true);
                }
            }
        }
    }

    /// Start a preset if none is showing, and move on when one has run its time.
    pub fn tick(&mut self, now: Instant) {
        if self.current.is_none() {
            let first = self.wanted.take().and_then(|n| self.presets.find(&n));
            match first {
                Some(i) => self.show(i, 0.0, true),
                None => self.next(0.0),
            }
        }
        if self.locked {
            self.engine.set_preset_length(None);
            return;
        }
        let due = match self.next_change {
            Some(at) => at,
            None => {
                let extra = self.seconds * RANDOM_SHARE * self.rng.below(1000) as f64 / 1000.0;
                let length = BLEND_AUTO + self.seconds + extra;
                self.engine.set_preset_length(Some(length));
                let at = now + std::time::Duration::from_secs_f64(length);
                self.next_change = Some(at);
                at
            }
        };
        if now >= due {
            self.next(BLEND_AUTO);
        }
    }

    /// The next preset: forward through those shown before when Previous went back, else a
    /// random one or the one listed after.
    fn next(&mut self, blend: f64) {
        let n = self.presets.len();
        if n == 0 {
            return;
        }
        if self.random && self.pos + 1 < self.history.len() {
            self.pos += 1;
            self.show(self.history[self.pos], blend, false);
            return;
        }
        let i = match self.current {
            Some(c) if !self.random => (c + 1) % n,
            Some(c) if n > 1 => {
                // Any other preset.
                let pick = self.rng.below(n - 1);
                if pick >= c { pick + 1 } else { pick }
            }
            _ if self.random => self.rng.below(n),
            _ => 0,
        };
        self.show(i, blend, true);
    }

    /// The preset shown before: in random order the one before in the history, in listed
    /// order the one listed before. It cuts in at once, as in MilkDrop.
    fn previous(&mut self) {
        let n = self.presets.len();
        if n == 0 {
            return;
        }
        if self.random {
            if self.pos > 0 {
                self.pos -= 1;
                self.show(self.history[self.pos], 0.0, false);
            }
        } else {
            let i = self.current.map_or(0, |c| (c + n - 1) % n);
            self.show(i, 0.0, true);
        }
    }

    /// Show preset `i`, blending over `blend` seconds, and add it to the history if `record`.
    /// A preset that cannot be read is skipped (its error is logged).
    fn show(&mut self, i: usize, blend: f64, record: bool) {
        let Some(preset) = self.presets.load(i) else {
            return;
        };
        if let Some(name) = self.presets.name(i) {
            log::info!("milkdrop: {name}");
        }
        self.engine.load(preset, blend);
        self.current = Some(i);
        self.wanted = None;
        self.next_change = None;
        if record {
            self.history.truncate(self.pos + 1);
            self.history.push(i);
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
            self.pos = self.history.len() - 1;
        }
    }

    /// The rate of the samples passed to [`Visualizer::push_audio`].
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.engine.set_sample_rate(rate);
    }

    /// Add interleaved stereo samples as they are played.
    pub fn push_audio(&mut self, samples: &[f32]) {
        self.engine.push_audio(samples);
    }

    /// Work out the frame at `now`, for a picture `size` pixels across and down.
    pub fn render(&mut self, now: Instant, size: (u32, u32)) -> &Frame {
        self.engine.render(now, size)
    }

    /// The engine, for drawing the frame it last worked out.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::Duration;

    use super::*;

    const BUILT_IN: &[(&str, &str)] = &[
        ("a", "[preset00]\n"),
        ("b", "[preset00]\n"),
        ("c", "[preset00]\n"),
        ("d", "[preset00]\n"),
    ];

    fn vis() -> Visualizer {
        Visualizer::new(Presets::new(BUILT_IN), 7)
    }

    #[test]
    fn it_starts_with_the_preset_asked_for() {
        let mut v = vis();
        v.start_with(Some("c".into()));
        assert_eq!(v.current_name(), Some("c"));
        v.tick(Instant::now());
        assert_eq!(v.current(), Some(2));
        // An unknown name starts with any preset.
        let mut v = vis();
        v.start_with(Some("nonesuch".into()));
        v.tick(Instant::now());
        assert!(v.current().is_some());
    }

    #[test]
    fn listed_order_goes_round() {
        let mut v = vis();
        v.set_random(false);
        v.tick(Instant::now());
        assert_eq!(v.current(), Some(0));
        for want in [1, 2, 3, 0] {
            v.change(Change::Next);
            assert_eq!(v.current(), Some(want));
        }
        v.change(Change::Previous);
        assert_eq!(v.current(), Some(3));
    }

    #[test]
    fn random_order_retraces_its_steps() {
        let mut v = vis();
        v.tick(Instant::now());
        let mut shown = vec![v.current().unwrap()];
        for _ in 0..5 {
            v.change(Change::HardCut);
            let now = v.current().unwrap();
            assert_ne!(
                Some(&now),
                shown.last(),
                "never the same preset twice running"
            );
            shown.push(now);
        }
        // Back three, then forward along the same way.
        for i in (2..5).rev() {
            v.change(Change::Previous);
            assert_eq!(v.current(), Some(shown[i]));
        }
        v.change(Change::Next);
        assert_eq!(v.current(), Some(shown[3]));
        // Choosing one forgets the way forward.
        v.change(Change::Select(0));
        v.change(Change::Previous);
        assert_eq!(v.current(), Some(shown[3]));
    }

    #[test]
    fn presets_change_by_themselves_unless_locked() {
        let t0 = Instant::now();
        let mut v = vis();
        v.set_seconds(5.0);
        v.tick(t0);
        let first = v.current();
        v.tick(t0 + Duration::from_secs(1));
        assert_eq!(v.current(), first);
        // 2.7 + 5 + at most 3.125 seconds.
        v.tick(t0 + Duration::from_secs(11));
        assert_ne!(v.current(), first);

        let second = v.current();
        v.set_locked(true);
        v.tick(t0 + Duration::from_secs(100));
        assert_eq!(v.current(), second);
        // Unlocked, the preset gets its whole time again.
        v.set_locked(false);
        v.tick(t0 + Duration::from_secs(101));
        v.tick(t0 + Duration::from_secs(108));
        assert_eq!(v.current(), second);
    }

    #[test]
    fn the_time_between_presets_stays_sensible() {
        let mut v = vis();
        v.set_seconds(0.0);
        assert!((v.seconds() - SECONDS_RANGE.0).abs() < 1e-9);
        v.set_seconds(f64::NAN);
        assert!((v.seconds() - SECONDS_RANGE.0).abs() < 1e-9);
        v.set_seconds(1e9);
        assert!((v.seconds() - SECONDS_RANGE.1).abs() < 1e-9);
    }

    #[test]
    fn a_new_list_keeps_the_preset_showing() {
        let mut v = vis();
        v.change(Change::Select(1));
        v.set_presets(Presets::new(&[
            ("x", "[preset00]\n"),
            ("b", "[preset00]\n"),
        ]));
        assert_eq!(v.current_name(), Some("b"));
        assert_eq!(v.current(), Some(0));
        v.set_presets(Presets::new(&[("x", "[preset00]\n")]));
        assert_eq!(v.current(), None);
        assert_eq!(v.current_name(), Some("b"), "kept in case it comes back");
        // The preset to start with is still started with, once there is a list.
        let mut v = vis();
        v.start_with(Some("c".into()));
        v.set_presets(Presets::new(BUILT_IN));
        assert_eq!(v.current(), None, "not showing until it is loaded");
        v.tick(Instant::now());
        assert_eq!(v.current_name(), Some("c"));
        assert!(v.engine().current().is_some());
    }

    #[test]
    fn nothing_happens_without_presets() {
        let mut v = Visualizer::new(Presets::default(), 1);
        v.tick(Instant::now());
        v.change(Change::Next);
        v.change(Change::Previous);
        v.change(Change::Select(3));
        assert_eq!(v.current(), None);
    }
}
