//! Pure geometry for detached windows: magnetic snapping and the
//! "which windows are docked together" graph. Kept free of egui state
//! so the rules are unit-testable.

use egui::{Rect, Vec2};

/// Distance (logical points) under which an edge snaps to another edge.
/// Winamp uses 10 px.
pub const SNAP_DISTANCE: f32 = 10.0;

/// Two edges closer than this count as touching (docked). Absorbs the
/// sub-point rounding the OS applies to window positions.
const TOUCH_EPS: f32 = 1.5;

/// True when `a` and `b` share an edge: one's side touches the other's
/// opposite side and they overlap along the other axis.
pub fn touching(a: Rect, b: Rect) -> bool {
    let overlap_x = a.min.x < b.max.x - TOUCH_EPS && b.min.x < a.max.x - TOUCH_EPS;
    let overlap_y = a.min.y < b.max.y - TOUCH_EPS && b.min.y < a.max.y - TOUCH_EPS;
    let vertical_edge =
        (a.max.x - b.min.x).abs() <= TOUCH_EPS || (b.max.x - a.min.x).abs() <= TOUCH_EPS;
    let horizontal_edge =
        (a.max.y - b.min.y).abs() <= TOUCH_EPS || (b.max.y - a.min.y).abs() <= TOUCH_EPS;
    (vertical_edge && overlap_y) || (horizontal_edge && overlap_x)
}

/// Indices of every rect transitively docked to `rects[root]`, root
/// included. `None` entries are hidden windows and never join a group.
pub fn docked_group(rects: &[Option<Rect>], root: usize) -> Vec<usize> {
    let mut group = vec![root];
    let mut i = 0;
    while i < group.len() {
        let Some(cur) = rects[group[i]] else {
            i += 1;
            continue;
        };
        for (j, r) in rects.iter().enumerate() {
            if let Some(r) = r
                && !group.contains(&j)
                && touching(cur, *r)
            {
                group.push(j);
            }
        }
        i += 1;
    }
    group
}

/// Correction to add to a move so that `moving` snaps onto the edges of
/// `targets` (window edges, outer and inner alignment) and `screen`.
/// Each axis snaps independently to its closest candidate.
pub fn snap_offset(moving: &[Rect], targets: &[Rect], screen: Option<Rect>) -> Vec2 {
    let mut best_x: Option<f32> = None;
    let mut best_y: Option<f32> = None;
    let consider = |best: &mut Option<f32>, d: f32| {
        if d.abs() <= SNAP_DISTANCE && best.is_none_or(|b| d.abs() < b.abs()) {
            *best = Some(d);
        }
    };
    for m in moving {
        for t in targets {
            // Only snap sideways when the rects are near each other on
            // the other axis — otherwise a window across the screen
            // would pull this one.
            let near_y = m.min.y <= t.max.y + SNAP_DISTANCE && t.min.y <= m.max.y + SNAP_DISTANCE;
            let near_x = m.min.x <= t.max.x + SNAP_DISTANCE && t.min.x <= m.max.x + SNAP_DISTANCE;
            if near_y {
                consider(&mut best_x, t.max.x - m.min.x); // left to right edge
                consider(&mut best_x, t.min.x - m.max.x); // right to left edge
                consider(&mut best_x, t.min.x - m.min.x); // align lefts
                consider(&mut best_x, t.max.x - m.max.x); // align rights
            }
            if near_x {
                consider(&mut best_y, t.max.y - m.min.y); // top to bottom edge
                consider(&mut best_y, t.min.y - m.max.y); // bottom to top edge
                consider(&mut best_y, t.min.y - m.min.y); // align tops
                consider(&mut best_y, t.max.y - m.max.y); // align bottoms
            }
        }
        if let Some(s) = screen {
            consider(&mut best_x, s.min.x - m.min.x);
            consider(&mut best_x, s.max.x - m.max.x);
            consider(&mut best_y, s.min.y - m.min.y);
            consider(&mut best_y, s.max.y - m.max.y);
        }
    }
    Vec2::new(best_x.unwrap_or(0.0), best_y.unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    #[test]
    fn stacked_windows_touch_and_group_transitively() {
        let main = r(0.0, 0.0, 275.0, 116.0);
        let eq = r(0.0, 116.0, 275.0, 116.0);
        let pl = r(0.0, 232.0, 275.0, 232.0);
        assert!(touching(main, eq));
        assert!(!touching(main, pl));
        let rects = [Some(main), Some(eq), Some(pl)];
        let mut g = docked_group(&rects, 0);
        g.sort();
        assert_eq!(g, vec![0, 1, 2]);
        // Hidden EQ breaks the chain.
        assert_eq!(docked_group(&[Some(main), None, Some(pl)], 0), vec![0]);
        // Corner contact only is not docking.
        assert!(!touching(main, r(275.0, 116.0, 10.0, 10.0)));
    }

    #[test]
    fn snaps_within_distance_only() {
        let main = r(0.0, 0.0, 275.0, 116.0);
        // EQ dragged 6 px below and 4 px right of the docked spot.
        let eq = r(4.0, 122.0, 275.0, 116.0);
        assert_eq!(snap_offset(&[eq], &[main], None), Vec2::new(-4.0, -6.0));
        // 30 px away: no pull.
        let far = r(0.0, 146.0, 275.0, 116.0);
        assert_eq!(snap_offset(&[far], &[main], None).y, 0.0);
        // Screen edge.
        let screen = r(0.0, 0.0, 1920.0, 1080.0);
        let near_edge = r(1640.0, 500.0, 275.0, 116.0);
        assert_eq!(snap_offset(&[near_edge], &[], Some(screen)).x, 5.0);
    }
}
