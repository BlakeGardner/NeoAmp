//! Playlist editing as Winamp 2.x did it, apart from the interface state: the new order of the
//! entries after a sort, a reversal, a shuffle or a move of the selection, and shuffle play.
//!
//! Orders are given the way [`crate::ui::Ui`] applies them: entry `i` of the new list is entry
//! `order[i]` of the old one.

use std::hash::{BuildHasher, RandomState};

use crate::audio::Location;
use crate::ui::Track;

/// What "Sort list" sorts by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    /// The title the playlist shows.
    Title,
    /// The file's name without its folder (for a stream, the last part of its URL).
    FileName,
    /// The whole path (for a stream, its URL).
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
        SortKey::FileName => track.location.name().to_lowercase(),
        SortKey::Path => match &track.location {
            Location::File(path) => path.to_string_lossy().to_lowercase(),
            Location::Url(url) => url.to_lowercase(),
        },
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
    /// The entry [`Shuffle::next`] will pick, drawn ahead by [`Shuffle::peek`] (so gapless
    /// playback can open it early), until the playlist or what has played changes.
    drawn: Option<Drawn>,
}

/// A pick drawn ahead, and what it was drawn for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Drawn {
    len: usize,
    current: Option<usize>,
    wrap: bool,
    pick: Option<usize>,
    /// Every entry had played: the pick starts a new round.
    new_round: bool,
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
            drawn: None,
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
        self.drawn = None;
    }

    /// Note that entry `i` is playing.
    pub fn mark(&mut self, i: usize) {
        self.drawn = None;
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
        let pick = self.peek(len, current, wrap);
        let Some(drawn) = self.drawn.take() else {
            return pick;
        };
        self.played.resize(len, false);
        if drawn.new_round {
            self.played.iter_mut().for_each(|p| *p = false);
        }
        // A single entry plays again without going into the history.
        if let (Some(p), Some(c)) = (pick, current)
            && p != c
        {
            self.history.push(c);
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        pick
    }

    /// What [`Shuffle::next`] will pick with the same arguments, without moving on: the same
    /// answer every time until the playlist or what has played changes.
    pub fn peek(&mut self, len: usize, current: Option<usize>, wrap: bool) -> Option<usize> {
        if let Some(d) = self.drawn
            && (d.len, d.current, d.wrap) == (len, current, wrap)
        {
            return d.pick;
        }
        let mut new_round = false;
        let fresh = |s: &Shuffle, new_round: bool, i: usize| {
            (new_round || !s.is_played(i)) && Some(i) != current
        };
        let mut left = (0..len).filter(|&i| fresh(self, false, i)).count();
        if left == 0 && wrap && len > 0 {
            new_round = true;
            left = (0..len).filter(|&i| fresh(self, true, i)).count();
        }
        let pick = if left > 0 {
            let k = self.rng.below(left);
            (0..len).filter(|&i| fresh(self, new_round, i)).nth(k)
        } else if new_round {
            // A single entry: it plays again.
            current.or(Some(0))
        } else {
            None
        };
        self.drawn = Some(Drawn {
            len,
            current,
            wrap,
            pick,
            new_round,
        });
        pick
    }

    /// The entry that played before the current one, if Previous can go back to one.
    pub fn back(&mut self) -> Option<usize> {
        self.history.pop()
    }

    /// The playlist was rearranged: `new_index[old]` is where each entry went.
    pub fn reorder(&mut self, new_index: &[usize]) {
        // The entry drawn to come next still comes next, wherever it went.
        self.drawn = self.drawn.and_then(|d| {
            let moved = |i: usize| new_index.get(i).copied();
            Some(Drawn {
                len: new_index.len(),
                current: match d.current {
                    Some(c) => Some(moved(c)?),
                    None => None,
                },
                pick: match d.pick {
                    Some(p) => Some(moved(p)?),
                    None => None,
                },
                ..d
            })
        });
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
        let kept_len = kept_at.iter().flatten().count();
        let kept = |i: usize| kept_at.get(i).copied().flatten();
        // The entry drawn to come next still comes next, unless it was removed: then another
        // is drawn. A removed current entry has none in its place.
        self.drawn = self.drawn.and_then(|d| {
            Some(Drawn {
                len: kept_len,
                current: d.current.and_then(kept),
                pick: match d.pick {
                    Some(p) => Some(kept(p)?),
                    None => None,
                },
                ..d
            })
        });
        let mut played = vec![false; kept_len];
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

    fn track(title: &str, path: &str) -> Track {
        Track {
            title: title.into(),
            seconds: None,
            location: Location::from_arg(path),
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
        // Streams sort by the end of their URL, or by the whole URL.
        let mixed = [
            track("x", "/m/b.mp3"),
            track("y", "HTTP://radio.example/a-stream?id=9"),
            track("z", "/m/c.mp3"),
        ];
        assert_eq!(sort_order(&mixed, SortKey::FileName), [1, 0, 2]);
        assert_eq!(sort_order(&mixed, SortKey::Path), [0, 2, 1]);
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
    fn a_peeked_shuffle_pick_holds_until_it_is_taken() {
        let mut s = Shuffle::new(Rng::with_seed(21));
        s.mark(4);
        let peeked = s.peek(30, Some(4), false);
        for _ in 0..5 {
            assert_eq!(
                s.peek(30, Some(4), false),
                peeked,
                "the same answer every frame"
            );
        }
        assert_eq!(s.next(30, Some(4), false), peeked);
        assert_eq!(s.back(), Some(4));
        // The playlist changing draws again.
        let first = s.peek(30, peeked, false);
        let mut s2 = s.clone();
        assert_eq!(s2.peek(31, peeked, false).is_some(), first.is_some());
        // Reordering keeps the same entry coming next; removing it draws another.
        let mut s3 = Shuffle::new(Rng::with_seed(8));
        let pick = s3.peek(4, Some(0), false).unwrap();
        let reversed = new_indices(&reverse_order(4));
        s3.reorder(&reversed);
        assert_eq!(s3.peek(4, Some(3), false), Some(reversed[pick]));
        let mut gone = vec![Some(0), Some(1), Some(2), Some(3)];
        gone[reversed[pick]] = None;
        let mut next = 0;
        for k in &mut gone {
            if k.is_some() {
                *k = Some(next);
                next += 1;
            }
        }
        s3.retain(&gone);
        assert!(s3.drawn.is_none(), "the drawn entry was removed");
        // At the end of a round, a peek with wrap starts the next one only once taken.
        let mut s = Shuffle::new(Rng::with_seed(2));
        s.mark(0);
        s.mark(1);
        assert_eq!(s.peek(2, Some(1), false), None);
        let pick = s.peek(2, Some(1), true);
        assert_eq!(pick, Some(0));
        assert!(s.is_played(0), "peeking did not start the new round");
        assert_eq!(s.next(2, Some(1), true), Some(0));
        assert!(!s.is_played(0), "taking it did");
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
