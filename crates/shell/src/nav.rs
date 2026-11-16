//! Pure focus-movement helpers, shared by every screen. Kept free of UI types so they can be unit-tested.

use cs_input::NavAction;

/// Moves focus in a horizontal or vertical list of `len` items. Stops at the ends (no wrap), as the
/// Xbox dashboard does.
pub fn list_move(focus: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (focus as isize + delta).clamp(0, len as isize - 1) as usize
}

/// Moves focus in a grid of `len` items laid out row-major with `cols` columns.
/// Left/right stop at row edges; down onto a shorter last row lands on its last item.
/// Returns `None` when the move would leave the grid (so the caller can move focus elsewhere, e.g. tabs).
pub fn grid_move(focus: usize, len: usize, cols: usize, action: NavAction) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let cols = cols.max(1);
    let (row, col) = (focus / cols, focus % cols);
    let last_row = (len - 1) / cols;
    match action {
        NavAction::Left if col > 0 => Some(focus - 1),
        NavAction::Right if col + 1 < cols && focus + 1 < len => Some(focus + 1),
        NavAction::Up if row > 0 => Some(focus - cols),
        NavAction::Down if row < last_row => Some((focus + cols).min(len - 1)),
        _ => None,
    }
}

/// An axis-aligned rectangle in design pixels (1920×1080 canvas).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn bottom(&self) -> f32 {
        self.y + self.h
    }

    fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Spatial navigation, like a console dashboard: from `from`, move to the nearest rect in the pressed
/// direction. Rects that overlap the current one across the movement axis (same row/column band) are
/// strongly preferred; among those the closest wins, ties broken by how well the centres line up.
pub fn spatial_move(rects: &[Rect], from: usize, dir: NavAction) -> Option<usize> {
    let cur = *rects.get(from)?;
    let (ccx, ccy) = cur.center();
    let mut best: Option<(f32, usize)> = None;
    for (i, r) in rects.iter().enumerate() {
        if i == from {
            continue;
        }
        let (cx, cy) = r.center();
        // (gap along the movement axis, overlap across it, centre offset across it)
        let (gap, overlap, offset) = match dir {
            NavAction::Right if cx > ccx + 0.5 => {
                (r.x - cur.right(), r.bottom().min(cur.bottom()) - r.y.max(cur.y), (cy - ccy).abs())
            }
            NavAction::Left if cx < ccx - 0.5 => {
                (cur.x - r.right(), r.bottom().min(cur.bottom()) - r.y.max(cur.y), (cy - ccy).abs())
            }
            NavAction::Down if cy > ccy + 0.5 => {
                (r.y - cur.bottom(), r.right().min(cur.right()) - r.x.max(cur.x), (cx - ccx).abs())
            }
            NavAction::Up if cy < ccy - 0.5 => {
                (cur.y - r.bottom(), r.right().min(cur.right()) - r.x.max(cur.x), (cx - ccx).abs())
            }
            _ => continue,
        };
        let score = gap.max(0.0) + offset * 0.3 + if overlap > 0.0 { 0.0 } else { 10_000.0 };
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use NavAction::*;

    /// The Home "hero" layout: big tile, 3 small squares to its right, and a lower row.
    fn hero() -> Vec<Rect> {
        vec![
            Rect::new(99.0, 258.0, 390.0, 387.0),  // 0 big
            Rect::new(536.0, 440.0, 200.0, 200.0), // 1 small
            Rect::new(752.0, 440.0, 200.0, 200.0), // 2 small
            Rect::new(968.0, 440.0, 200.0, 200.0), // 3 small
            Rect::new(99.0, 692.0, 386.0, 216.0),  // 4 My games & apps
            Rect::new(536.0, 692.0, 416.0, 216.0), // 5 wide
            Rect::new(967.0, 692.0, 416.0, 216.0), // 6 wide
        ]
    }

    #[test]
    fn spatial_hero_layout() {
        let r = hero();
        assert_eq!(spatial_move(&r, 0, Right), Some(1));
        assert_eq!(spatial_move(&r, 0, Down), Some(4));
        assert_eq!(spatial_move(&r, 0, Up), None);
        assert_eq!(spatial_move(&r, 1, Left), Some(0));
        assert_eq!(spatial_move(&r, 3, Down), Some(6)); // x-overlap with the second wide tile
        assert_eq!(spatial_move(&r, 2, Down), Some(5));
        assert_eq!(spatial_move(&r, 4, Right), Some(5));
        assert_eq!(spatial_move(&r, 5, Up), Some(1)); // closest centre among overlapping squares
        assert_eq!(spatial_move(&r, 6, Up), Some(3));
        assert_eq!(spatial_move(&r, 6, Right), None);
    }

    #[test]
    fn list_clamps() {
        assert_eq!(list_move(0, 5, -1), 0);
        assert_eq!(list_move(4, 5, 1), 4);
        assert_eq!(list_move(2, 5, 1), 3);
        assert_eq!(list_move(0, 0, 1), 0);
    }

    #[test]
    fn grid_edges() {
        // 7 items, 3 columns:
        // 0 1 2
        // 3 4 5
        // 6
        assert_eq!(grid_move(0, 7, 3, Left), None);
        assert_eq!(grid_move(0, 7, 3, Up), None);
        assert_eq!(grid_move(2, 7, 3, Right), None);
        assert_eq!(grid_move(1, 7, 3, Right), Some(2));
        assert_eq!(grid_move(1, 7, 3, Down), Some(4));
        assert_eq!(grid_move(5, 7, 3, Down), Some(6)); // shorter last row
        assert_eq!(grid_move(6, 7, 3, Right), None);
        assert_eq!(grid_move(6, 7, 3, Down), None);
        assert_eq!(grid_move(6, 7, 3, Up), Some(3));
        assert_eq!(grid_move(0, 0, 3, Down), None);
    }
}
