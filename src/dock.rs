//! Winamp-style magnetic docking: windows snap to each other's edges and to the screen,
//! and windows touching the main window travel along with it.
//!
//! Everything here is pure geometry in physical pixels; the window layer feeds it the
//! current rectangles and applies the result.

use crate::canvas::Rect;

/// Edges closer than this snap together (Winamp's default "snap at 10 pixels").
pub const SNAP_DISTANCE: i32 = 10;

/// Half-open ranges `[a0, a1)` and `[b0, b1)` overlap, or come within `slack` of doing so.
fn near_overlap(a0: i32, a1: i32, b0: i32, b1: i32, slack: i32) -> bool {
    a0 < b1 + slack && b0 < a1 + slack
}

/// The `(dx, dy)` to add to `moving` so it snaps to the nearest edges of `targets` and,
/// when given, to the `screen` bounds.
///
/// Each axis snaps independently to its closest candidate within `snap`; `(0, 0)` means
/// nothing is close enough. Horizontal candidates only count when the rectangles are
/// (nearly) side by side vertically, and vice versa, so a window passing far above another
/// does not get pulled sideways.
#[must_use]
pub fn snap_delta(moving: Rect, targets: &[Rect], screen: Option<Rect>, snap: i32) -> (i32, i32) {
    let mut best_dx: Option<i32> = None;
    let mut best_dy: Option<i32> = None;
    let consider = |best: &mut Option<i32>, d: i32| {
        if d.abs() <= snap && best.is_none_or(|b| d.abs() < b.abs()) {
            *best = Some(d);
        }
    };
    for t in targets {
        if near_overlap(moving.y, moving.bottom(), t.y, t.bottom(), snap) {
            consider(&mut best_dx, t.right() - moving.x); // dock to its right side
            consider(&mut best_dx, t.x - moving.right()); // dock to its left side
            consider(&mut best_dx, t.x - moving.x); // align left edges
            consider(&mut best_dx, t.right() - moving.right()); // align right edges
        }
        if near_overlap(moving.x, moving.right(), t.x, t.right(), snap) {
            consider(&mut best_dy, t.bottom() - moving.y); // dock below it
            consider(&mut best_dy, t.y - moving.bottom()); // dock above it
            consider(&mut best_dy, t.y - moving.y); // align top edges
            consider(&mut best_dy, t.bottom() - moving.bottom()); // align bottom edges
        }
    }
    if let Some(s) = screen {
        consider(&mut best_dx, s.x - moving.x);
        consider(&mut best_dx, s.right() - moving.right());
        consider(&mut best_dy, s.y - moving.y);
        consider(&mut best_dy, s.bottom() - moving.bottom());
    }
    (best_dx.unwrap_or(0), best_dy.unwrap_or(0))
}

/// Two rectangles sit edge to edge (within a pixel) and overlap along that shared edge.
fn touching(a: Rect, b: Rect) -> bool {
    let side = ((a.right() - b.x).abs() <= 1 || (b.right() - a.x).abs() <= 1)
        && near_overlap(a.y, a.bottom(), b.y, b.bottom(), 0);
    let stack = ((a.bottom() - b.y).abs() <= 1 || (b.bottom() - a.y).abs() <= 1)
        && near_overlap(a.x, a.right(), b.x, b.right(), 0);
    side || stack
}

/// Indices of the rectangles attached to `rects[root]`, directly or through other attached
/// rectangles. `root` itself is not included.
#[must_use]
pub fn attached(root: usize, rects: &[Rect]) -> Vec<usize> {
    let mut found = vec![false; rects.len()];
    let Some(slot) = found.get_mut(root) else {
        return Vec::new();
    };
    *slot = true;
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        for (j, r) in rects.iter().enumerate() {
            if !found[j] && touching(rects[i], *r) {
                found[j] = true;
                stack.push(j);
            }
        }
    }
    (0..rects.len())
        .filter(|&j| j != root && found[j])
        .collect()
}

/// Indices of the rectangles hanging below `rects[root]`: those whose top edge lies on its
/// bottom edge (within a pixel) and that overlap it sideways, then those hanging below them,
/// and so on. When the root window grows or shrinks in height, as in windowshade mode, these
/// move by as much so that they stay docked, as Webamp does. Windows docked beside it stay.
#[must_use]
pub fn hanging_below(root: usize, rects: &[Rect]) -> Vec<usize> {
    let mut found = vec![false; rects.len()];
    let Some(slot) = found.get_mut(root) else {
        return Vec::new();
    };
    *slot = true;
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        let above = rects[i];
        for (j, r) in rects.iter().enumerate() {
            if !found[j]
                && (r.y - above.bottom()).abs() <= 1
                && near_overlap(above.x, above.right(), r.x, r.right(), 0)
            {
                found[j] = true;
                stack.push(j);
            }
        }
    }
    (0..rects.len())
        .filter(|&j| j != root && found[j])
        .collect()
}

/// New top-left corner for a window being dragged by hand.
///
/// `dragged` is its current rectangle, `pointer` the pointer's screen position and `grab`
/// the pointer's offset inside the window when the drag began. `followers` are the attached
/// windows as `(dx, dy, w, h)` relative to the dragged window's corner; they move as one block,
/// and the block as a whole snaps to `statics` and the `screen`.
#[must_use]
pub fn drag_target(
    dragged: Rect,
    followers: &[(i32, i32, i32, i32)],
    pointer: (i32, i32),
    grab: (i32, i32),
    statics: &[Rect],
    screen: Option<Rect>,
    snap: i32,
) -> (i32, i32) {
    let (nx, ny) = (pointer.0 - grab.0, pointer.1 - grab.1);
    let mut rects = Vec::with_capacity(followers.len() + 1);
    rects.push(Rect::new(nx, ny, dragged.w, dragged.h));
    rects.extend(
        followers
            .iter()
            .map(|&(dx, dy, w, h)| Rect::new(nx + dx, ny + dy, w, h)),
    );
    let (sx, sy) = bounding(&rects).map_or((0, 0), |b| snap_delta(b, statics, screen, snap));
    (nx + sx, ny + sy)
}

/// The smallest rectangle containing all of `rects`; `None` when empty.
#[must_use]
pub fn bounding(rects: &[Rect]) -> Option<Rect> {
    let first = *rects.first()?;
    Some(rects.iter().skip(1).fold(first, |acc, r| {
        let x = acc.x.min(r.x);
        let y = acc.y.min(r.y);
        Rect::new(
            x,
            y,
            acc.right().max(r.right()) - x,
            acc.bottom().max(r.bottom()) - y,
        )
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect::new(0, 0, 275, 116);

    #[test]
    fn snaps_to_the_right_side_of_a_neighbour() {
        let moving = Rect::new(275 + 7, 20, 275, 116);
        assert_eq!(snap_delta(moving, &[MAIN], None, SNAP_DISTANCE), (-7, 0));
    }

    #[test]
    fn snaps_below_a_neighbour_and_aligns_the_left_edge() {
        let moving = Rect::new(3, 116 + 5, 275, 116);
        assert_eq!(snap_delta(moving, &[MAIN], None, SNAP_DISTANCE), (-3, -5));
    }

    #[test]
    fn ignores_edges_that_are_too_far_or_not_alongside() {
        assert_eq!(
            snap_delta(Rect::new(0, 500, 275, 116), &[MAIN], None, SNAP_DISTANCE),
            (0, 0)
        );
        // Same x, but 100px above: left edges align only when nearly adjacent vertically.
        assert_eq!(
            snap_delta(Rect::new(4, -216, 275, 116), &[MAIN], None, SNAP_DISTANCE),
            (0, 0)
        );
        assert_eq!(
            snap_delta(
                Rect::new(275 + 11, 0, 275, 116),
                &[MAIN],
                None,
                SNAP_DISTANCE
            ),
            (0, 0)
        );
    }

    #[test]
    fn snaps_to_the_screen_edges() {
        let screen = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            snap_delta(
                Rect::new(-4, -6, 275, 116),
                &[],
                Some(screen),
                SNAP_DISTANCE
            ),
            (4, 6)
        );
        let moving = Rect::new(1920 - 275 + 2, 1080 - 116 + 9, 275, 116);
        assert_eq!(
            snap_delta(moving, &[], Some(screen), SNAP_DISTANCE),
            (-2, -9)
        );
    }

    #[test]
    fn prefers_the_nearest_candidate_per_axis() {
        let other = Rect::new(600, 0, 275, 116);
        // 8px from MAIN's right side, 2px from `other`'s left side.
        let moving = Rect::new(275 + 8, 0, 600 - 275 - 10, 116);
        let (dx, _) = snap_delta(moving, &[MAIN, other], None, SNAP_DISTANCE);
        assert_eq!(dx, 2);
    }

    #[test]
    fn attachment_is_transitive_and_requires_touching() {
        let eq = Rect::new(0, 116, 275, 116);
        let pl = Rect::new(0, 232, 275, 116);
        let far = Rect::new(900, 900, 275, 116);
        let gap = Rect::new(0, 232 + 116 + 2, 275, 116);
        let rects = [MAIN, eq, pl, far, gap];
        assert_eq!(attached(0, &rects), vec![1, 2]);
        assert_eq!(attached(1, &rects), vec![0, 2]);
        assert_eq!(attached(3, &rects), Vec::<usize>::new());
        assert_eq!(attached(99, &rects), Vec::<usize>::new());
        // Side by side counts too.
        let beside = Rect::new(275, 10, 100, 50);
        assert_eq!(attached(0, &[MAIN, beside]), vec![1]);
    }

    #[test]
    fn windows_hanging_below_are_found_down_the_stack_only() {
        let eq = Rect::new(0, 116, 275, 116);
        let pl = Rect::new(0, 232 + 1, 275, 116); // a pixel off still counts
        let beside = Rect::new(275, 0, 275, 116);
        let under_beside = Rect::new(275, 116, 275, 116);
        let apart = Rect::new(0, 400, 275, 116);
        let rects = [MAIN, eq, pl, beside, under_beside, apart];
        assert_eq!(hanging_below(0, &rects), vec![1, 2]);
        assert_eq!(hanging_below(1, &rects), vec![2]);
        assert_eq!(hanging_below(3, &rects), vec![4]);
        assert_eq!(hanging_below(2, &rects), Vec::<usize>::new());
        assert_eq!(hanging_below(9, &rects), Vec::<usize>::new());
        // Touching only at a corner is not hanging below.
        let corner = Rect::new(275, 116, 100, 50);
        assert_eq!(hanging_below(0, &[MAIN, corner]), Vec::<usize>::new());
    }

    #[test]
    fn dragging_a_cluster_snaps_the_whole_block() {
        // Main dragged with the equalizer hanging below it; the playlist sits 8px further down.
        let eq = (0, 116, 275, 116);
        let playlist = Rect::new(0, 232 + 8, 275, 116);
        // Pointer at (50, 5) with grab (50, 5): the cluster would land at (0, 0).
        let target = drag_target(
            MAIN,
            &[eq],
            (50, 5),
            (50, 5),
            &[playlist],
            None,
            SNAP_DISTANCE,
        );
        assert_eq!(target, (0, 8));
        // Far from anything: the pointer simply carries the window.
        let target = drag_target(
            MAIN,
            &[eq],
            (1050, 505),
            (50, 5),
            &[playlist],
            None,
            SNAP_DISTANCE,
        );
        assert_eq!(target, (1000, 500));
    }

    #[test]
    fn bounding_box_covers_all_rectangles() {
        assert_eq!(bounding(&[]), None);
        let b = bounding(&[
            MAIN,
            Rect::new(-10, 116, 50, 50),
            Rect::new(100, 100, 300, 10),
        ])
        .unwrap();
        assert_eq!(b, Rect::new(-10, 0, 410, 166));
    }
}
