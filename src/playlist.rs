//! Playlist editing as Winamp 2.x did it, apart from the interface state: the new order of the
//! entries after a sort, a reversal, a shuffle or a move of the selection, and shuffle play.
//!
//! Orders are given the way [`crate::ui::Ui`] applies them: entry `i` of the new list is entry
//! `order[i]` of the old one.

use std::hash::{BuildHasher, RandomState};

use crate::ui::Track;

/// What "Sort list" sorts by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    /// The title the playlist shows.
    Title,
    /// The file's name without its folder.
    FileName,
    /// The whole path.
    Path,
}

/// The order that sorts `tracks` by `key`, ignoring case. Entries that compare equal keep their
/// order.
#[must_use]
pub fn sort_order(tracks: &[Track], key: SortKey) -> Vec<usize> {
    let mut keyed: Vec<(String, usize)> = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (sort_text(t, key), i))
        .collect();
    // The index breaks ties, so the sort is stable.
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, i)| i).collect()
}

fn sort_text(track: &Track, key: SortKey) -> String {
    match key {
        SortKey::Title => track.title.to_lowercase(),
        SortKey::FileName => track
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default(),
        SortKey::Path => track.path.to_string_lossy().to_lowercase(),
    }
}

/// The order that reverses a list of `n` entries.
#[must_use]
pub fn reverse_order(n: usize) -> Vec<usize> {
    (0..n).rev().collect()
}

/// A random order of `n` entries, every one equally likely.
#[must_use]
pub fn random_order(n: usize, rng: &mut Rng) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    // Fisher-Yates.
    for i in (1..n).rev() {
        order.swap(i, rng.below(i + 1));
    }
    order
}

/// The order that moves the selected entries `delta` places down (up when negative), together,
/// as dragging them or Alt+Up and Alt+Down did in Winamp. The other entries close up around
/// them in their own order. The move stops at either end of the list, so it may be shorter
/// than asked: returns the order and how far the entries moved, or `None` if they cannot move
/// or nothing is selected.
#[must_use]
pub fn move_order(selected: &[bool], delta: isize) -> Option<(Vec<usize>, isize)> {
    let first = selected.iter().position(|&s| s)?;
    let last = selected.iter().rposition(|&s| s)?;
    let room_up = isize::try_from(first).ok()?;
    let room_down = isize::try_from(selected.len() - 1 - last).ok()?;
    let delta = delta.clamp(-room_up, room_down);
    if delta == 0 {
        return None;
    }
    let mut slots: Vec<Option<usize>> = vec![None; selected.len()];
    for (i, _) in selected.iter().enumerate().filter(|(_, s)| **s) {
        // Within the list: the clamp above kept the first and last selected entries in it.
        let to = i.checked_add_signed(delta)?;
        *slots.get_mut(to)? = Some(i);
    }
    let mut rest = (0..selected.len()).filter(|&i| !selected[i]);
    for slot in slots.iter_mut().filter(|s| s.is_none()) {
        *slot = rest.next();
    }
    Some((slots.into_iter().flatten().collect(), delta))
}

/// Where each entry went: `new_index[old] = new` for an order as described in the module docs.
#[must_use]
pub fn new_indices(order: &[usize]) -> Vec<usize> {
    let mut new_index = vec![0; order.len()];
    for (new, &old) in order.iter().enumerate() {
        if let Some(slot) = new_index.get_mut(old) {
            *slot = new;
        }
    }
    new_index
}

/// A small, fast random number generator (SplitMix64) for shuffling. Not for anything that
/// needs to be unpredictable.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        Self::new()
    }
}

impl Rng {
    /// Seeded differently on every run.
    #[must_use]
    pub fn new() -> Rng {
        Rng(RandomState::new().hash_one(0x5EED_u32))
    }

    /// Always the same sequence for the same seed (for tests).
    #[must_use]
    pub const fn with_seed(seed: u64) -> Rng {
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`; 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        // Lemire's multiply-shift: the high half of a 64x64-bit product is below `n`.
        let wide = (u128::from(self.next_u64()) * n as u128) >> 64;
        usize::try_from(wide).unwrap_or(0)
    }
}

/// Shuffle play: every entry plays once, in random order, before any plays again, and Previous
/// retraces the order they played in.
#[derive(Clone, Debug, Default)]
pub struct Shuffle {
    /// Which entries have played in this round, by playlist index. Entries past its end have
    /// not.
    played: Vec<bool>,
    /// The entries played before the current one, most recent last, for Previous.
    history: Vec<usize>,
    rng: Rng,
}

/// How many entries Previous can retrace.
const HISTORY: usize = 1000;

impl Shuffle {
    #[must_use]
    pub fn new(rng: Rng) -> Shuffle {
        Shuffle {
            played: Vec::new(),
            history: Vec::new(),
            rng,
        }
    }

    /// The random numbers behind shuffle play, also used to randomize the list.
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// Start afresh: nothing has played, and Previous has nothing to go back to.
    pub fn reset(&mut self) {
        self.played.clear();
        self.history.clear();
    }

    /// Note that entry `i` is playing.
    pub fn mark(&mut self, i: usize) {
        if self.played.len() <= i {
            self.played.resize(i + 1, false);
        }
        self.played[i] = true;
    }

    fn is_played(&self, i: usize) -> bool {
        self.played.get(i).copied().unwrap_or(false)
    }

    /// The entry to play after `current` in a list of `len`: one that has not played in this
    /// round, at random. Once all have, a new round starts if `wrap` is set (with any entry but
    /// `current`, if there is another); otherwise there is none. Remembers `current` for
    /// [`Shuffle::back`].
    pub fn next(&mut self, len: usize, current: Option<usize>, wrap: bool) -> Option<usize> {
        if len == 0 {
            return None;
        }
        self.played.resize(len, false);
        let fresh = |s: &Shuffle, i: usize| !s.is_played(i) && Some(i) != current;
        let mut left = (0..len).filter(|&i| fresh(self, i)).count();
        if left == 0 {
            if !wrap {
                return None;
            }
            self.played.iter_mut().for_each(|p| *p = false);
            left = (0..len).filter(|&i| fresh(self, i)).count();
            if left == 0 {
                // A single entry: it plays again.
                return current.or(Some(0));
            }
        }
        let k = self.rng.below(left);
        let pick = (0..len).filter(|&i| fresh(self, i)).nth(k)?;
        if let Some(c) = current {
            self.history.push(c);
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        Some(pick)
    }

    /// The entry that played before the current one, if Previous can go back to one.
    pub fn back(&mut self) -> Option<usize> {
        self.history.pop()
    }

    /// The playlist was rearranged: `new_index[old]` is where each entry went.
    pub fn reorder(&mut self, new_index: &[usize]) {
        let mut played = vec![false; new_index.len()];
        for (old, &new) in new_index.iter().enumerate() {
            if self.is_played(old)
                && let Some(p) = played.get_mut(new)
            {
                *p = true;
            }
        }
        self.played = played;
        for h in &mut self.history {
            *h = new_index.get(*h).copied().unwrap_or(usize::MAX);
        }
        self.history.retain(|&h| h != usize::MAX);
    }

    /// Entries were removed from the playlist: `kept_at[old]` is where each kept entry went,
    /// and `None` for the removed ones.
    pub fn retain(&mut self, kept_at: &[Option<usize>]) {
        let kept = kept_at.iter().flatten().count();
        let mut played = vec![false; kept];
        for (old, new) in kept_at.iter().enumerate() {
            if let Some(new) = *new
                && self.is_played(old)
                && let Some(p) = played.get_mut(new)
            {
                *p = true;
            }
        }
        self.played = played;
        self.history = self
            .history
            .iter()
            .filter_map(|&h| kept_at.get(h).copied().flatten())
            .collect();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn track(title: &str, path: &str) -> Track {
        Track {
            title: title.into(),
            seconds: None,
            path: PathBuf::from(path),
        }
    }

    fn apply<T: Clone>(items: &[T], order: &[usize]) -> Vec<T> {
        order.iter().map(|&i| items[i].clone()).collect()
    }

    #[test]
    fn sorting_ignores_case_and_keeps_ties_in_order() {
        let tracks = [
            track("beta", "/z/2.mp3"),
            track("Alpha", "/y/3.mp3"),
            track("alpha", "/x/1.mp3"),
        ];
        assert_eq!(sort_order(&tracks, SortKey::Title), [1, 2, 0]);
        assert_eq!(sort_order(&tracks, SortKey::FileName), [2, 0, 1]);
        assert_eq!(sort_order(&tracks, SortKey::Path), [2, 1, 0]);
        assert_eq!(sort_order(&[], SortKey::Title), Vec::<usize>::new());
    }

    #[test]
    fn reversing_and_randomizing_keep_every_entry() {
        assert_eq!(reverse_order(4), [3, 2, 1, 0]);
        let mut rng = Rng::with_seed(7);
        let mut order = random_order(50, &mut rng);
        assert_ne!(order, (0..50).collect::<Vec<_>>());
        order.sort_unstable();
        assert_eq!(order, (0..50).collect::<Vec<_>>());
        assert!(random_order(0, &mut rng).is_empty());
    }

    #[test]
    fn moving_the_selection_keeps_its_shape_and_stops_at_the_ends() {
        let list = ["a", "b", "c", "d", "e"];
        let sel = [false, true, false, true, false]; // b and d
        let (order, moved) = move_order(&sel, 1).unwrap();
        assert_eq!(moved, 1);
        assert_eq!(apply(&list, &order), ["a", "c", "b", "e", "d"]);
        // Two up is only one: b is near the top.
        let (order, moved) = move_order(&sel, -2).unwrap();
        assert_eq!(moved, -1);
        assert_eq!(apply(&list, &order), ["b", "a", "d", "c", "e"]);
        assert_eq!(move_order(&[true, false], -1), None, "already at the top");
        assert_eq!(move_order(&[false, false], 1), None, "nothing selected");
        assert_eq!(move_order(&[], 1), None);
    }

    #[test]
    fn new_indices_invert_an_order() {
        assert_eq!(new_indices(&[2, 0, 1]), [1, 2, 0]);
    }

    #[test]
    fn random_numbers_stay_below_the_bound() {
        let mut rng = Rng::with_seed(1);
        let mut seen = [false; 6];
        for _ in 0..600 {
            let n = rng.below(6);
            seen[n] = true;
        }
        assert!(seen.iter().all(|&s| s), "{seen:?}");
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.below(1), 0);
    }

    #[test]
    fn shuffle_plays_everything_once_per_round() {
        let mut s = Shuffle::new(Rng::with_seed(3));
        s.mark(0);
        let mut current = 0;
        let mut seen = vec![0];
        for _ in 1..8 {
            current = s.next(8, Some(current), false).unwrap();
            s.mark(current);
            seen.push(current);
        }
        seen.sort_unstable();
        assert_eq!(seen, (0..8).collect::<Vec<_>>());
        assert_eq!(s.next(8, Some(current), false), None, "round over");
        let again = s.next(8, Some(current), true).unwrap();
        assert_ne!(
            again, current,
            "a new round avoids repeating the last entry"
        );
        assert_eq!(s.next(1, Some(0), true), Some(0), "a single entry repeats");
    }

    #[test]
    fn previous_retraces_the_shuffled_order() {
        let mut s = Shuffle::new(Rng::with_seed(9));
        let a = s.next(5, Some(2), true).unwrap();
        let b = s.next(5, Some(a), true).unwrap();
        assert_ne!(a, b);
        assert_eq!(s.back(), Some(a));
        assert_eq!(s.back(), Some(2));
        assert_eq!(s.back(), None);
    }

    #[test]
    fn shuffle_memory_follows_reorders_and_removals() {
        let mut s = Shuffle::new(Rng::with_seed(5));
        s.mark(0);
        s.history = vec![0, 2];
        // Reverse four entries: 0 -> 3, 2 -> 1.
        s.reorder(&new_indices(&reverse_order(4)));
        assert!(s.is_played(3) && !s.is_played(0));
        assert_eq!(s.history, [3, 1]);
        // Remove entry 1.
        s.retain(&[Some(0), None, Some(1), Some(2)]);
        assert!(s.is_played(2));
        assert_eq!(s.history, [2]);
    }
}
