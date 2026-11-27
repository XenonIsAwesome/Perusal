//! Split-pane layout for batched files: a tiling tree with resizable dividers that snap to halves, thirds,
//! quarters, to dividers in neighbouring rows/columns, and (via corner handles) to both axes at once.
//! Everything works in unit coordinates (0..1 of the pane area); the UI converts to pixels.
use serde::{Deserialize, Serialize};

/// How a batch's files are arranged. `Auto` is side by side for two files and a grid for more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arrangement { #[default] Auto, Columns, Rows, Grid }

impl Arrangement {
    pub const ALL: [Arrangement; 4] = [Arrangement::Auto, Arrangement::Columns, Arrangement::Rows, Arrangement::Grid];
    pub fn label(self) -> &'static str { match self { Arrangement::Auto => "Automatic", Arrangement::Columns => "Side by side", Arrangement::Rows => "Stacked rows", Arrangement::Grid => "Grid" } }
}

/// Whether the files of a batch are shown in split panes or one after the other in a single scrolling column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchView { #[default] Panes, Stacked }

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(usize),
    /// `cols`: children sit side by side (a vertical divider between them); otherwise they are stacked.
    Split { cols: bool, fracs: Vec<f32>, kids: Vec<Node> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DividerId { pub path: Vec<usize>, pub index: usize }

#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub id: DividerId,
    /// A vertical line (between side-by-side panes).
    pub vertical: bool,
    /// Position along the axis it moves on (x for vertical lines, y for horizontal ones).
    pub pos: f32,
    /// Extent along the other axis.
    pub span: (f32, f32),
}

/// Where a vertical and a horizontal divider meet; dragging it moves both.
#[derive(Debug, Clone, PartialEq)]
pub struct Corner { pub v: DividerId, pub h: DividerId, pub x: f32, pub y: f32 }

/// The fractions a divider snaps to (of the whole pane area): halves, thirds and quarters.
pub const SNAP_FRACTIONS: [f32; 5] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75];

#[derive(Debug, Clone, PartialEq)]
pub struct Layout { pub n: usize, pub arrangement: Arrangement, /// Set once panes were dragged into a hand-made arrangement.
    pub custom: bool, root: Node }

/// The side of a pane something is dropped on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge { Left, Right, Top, Bottom }

/// Where on a pane a dragged pane lands: an edge splits the target, the middle swaps the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop { Swap, Split(Edge) }

/// Which drop a point inside a pane (unit coordinates of that pane) means: the outer 28 % on a side splits that side, the rest swaps.
pub fn drop_zone(rx: f32, ry: f32) -> Drop {
    const E: f32 = 0.28;
    let d = [(rx, Edge::Left), (1.0 - rx, Edge::Right), (ry, Edge::Top), (1.0 - ry, Edge::Bottom)];
    let (dist, edge) = d.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
    if dist < E { Drop::Split(edge) } else { Drop::Swap }
}

fn equal(n: usize) -> Vec<f32> { vec![1.0 / n as f32; n] }

fn node_at<'a>(mut n: &'a Node, path: &[usize]) -> Option<&'a Node> {
    for &i in path { match n { Node::Split { kids, .. } => n = kids.get(i)?, Node::Leaf(_) => return None } }
    Some(n)
}
fn node_at_mut<'a>(mut n: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    for &i in path { match n { Node::Split { kids, .. } => n = kids.get_mut(i)?, Node::Leaf(_) => return None } }
    Some(n)
}

impl Layout {
    /// A layout for `n` panes. Zero panes behave like one.
    pub fn new(n: usize, arrangement: Arrangement) -> Layout {
        let n = n.max(1);
        let leaves = |r: std::ops::Range<usize>| -> Vec<Node> { r.map(Node::Leaf).collect() };
        let arr = match arrangement { Arrangement::Auto => if n <= 2 { Arrangement::Columns } else { Arrangement::Grid }, a => a };
        let root = if n == 1 { Node::Leaf(0) } else { match arr {
            Arrangement::Columns | Arrangement::Auto => Node::Split { cols: true, fracs: equal(n), kids: leaves(0..n) },
            Arrangement::Rows => Node::Split { cols: false, fracs: equal(n), kids: leaves(0..n) },
            Arrangement::Grid => {
                let per_row = (n as f32).sqrt().ceil() as usize; let nrows = n.div_ceil(per_row);
                let rows: Vec<Node> = (0..nrows).map(|r| {
                    let (a, b) = (r * per_row, ((r + 1) * per_row).min(n));
                    if b - a == 1 { Node::Leaf(a) } else { Node::Split { cols: true, fracs: equal(b - a), kids: leaves(a..b) } }
                }).collect();
                if nrows == 1 { rows.into_iter().next().unwrap() } else { Node::Split { cols: false, fracs: equal(nrows), kids: rows } }
            }
        } };
        Layout { n, arrangement, custom: false, root }
    }
    /// True when this layout was built for `n` panes and `arrangement`, so user resizing can be kept.
    pub fn fits(&self, n: usize, arrangement: Arrangement) -> bool { self.n == n.max(1) && (self.custom || self.arrangement == arrangement) }

    fn walk(node: &Node, r: Rect, path: &mut Vec<usize>, f: &mut impl FnMut(&Node, Rect, &[usize])) {
        f(node, r, path);
        if let Node::Split { cols, fracs, kids } = node {
            let mut at = 0.0;
            for (i, (k, fr)) in kids.iter().zip(fracs).enumerate() {
                let kr = if *cols { Rect { x: r.x + at * r.w, y: r.y, w: fr * r.w, h: r.h } } else { Rect { x: r.x, y: r.y + at * r.h, w: r.w, h: fr * r.h } };
                at += fr; path.push(i); Self::walk(k, kr, path, f); path.pop();
            }
        }
    }

    /// Pane index and its rectangle, in pane order.
    pub fn rects(&self) -> Vec<(usize, Rect)> {
        let mut out = vec![];
        Self::walk(&self.root, Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, &mut vec![], &mut |n, r, _| if let Node::Leaf(i) = n { out.push((*i, r)); });
        out.sort_by_key(|x| x.0); out
    }
    fn region(&self, path: &[usize]) -> Rect {
        let mut found = Rect::default();
        Self::walk(&self.root, Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, &mut vec![], &mut |_, r, p| if p == path { found = r; });
        found
    }

    /// Every draggable divider.
    pub fn dividers(&self) -> Vec<Divider> {
        let mut out = vec![];
        Self::walk(&self.root, Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, &mut vec![], &mut |n, r, path| {
            if let Node::Split { cols, fracs, .. } = n {
                let mut at = 0.0;
                for (i, fr) in fracs.iter().take(fracs.len() - 1).enumerate() {
                    at += fr;
                    let id = DividerId { path: path.to_vec(), index: i };
                    out.push(if *cols { Divider { id, vertical: true, pos: r.x + at * r.w, span: (r.y, r.y + r.h) } } else { Divider { id, vertical: false, pos: r.y + at * r.h, span: (r.x, r.x + r.w) } });
                }
            }
        });
        out
    }

    /// Places where a vertical divider ends on a horizontal one (a T or a cross).
    pub fn corners(&self) -> Vec<Corner> {
        let ds = self.dividers(); let eps = 1e-3; let mut out = vec![];
        for v in ds.iter().filter(|d| d.vertical) {
            for h in ds.iter().filter(|d| !d.vertical) {
                let touches = (v.span.0 - h.pos).abs() < eps || (v.span.1 - h.pos).abs() < eps;
                if touches && v.pos > h.span.0 + eps && v.pos < h.span.1 - eps { out.push(Corner { v: v.id.clone(), h: h.id.clone(), x: v.pos, y: h.pos }); }
            }
        }
        out
    }

    /// Candidate positions for divider `id` to snap to: fractions of the whole area, of its own region, and other dividers on the same axis.
    fn snap_points(&self, id: &DividerId, vertical: bool) -> Vec<f32> {
        let reg = self.region(&id.path); let (start, len) = if vertical { (reg.x, reg.w) } else { (reg.y, reg.h) };
        let mut pts: Vec<f32> = SNAP_FRACTIONS.to_vec();
        pts.extend(SNAP_FRACTIONS.iter().map(|f| start + f * len));
        pts.extend(self.dividers().into_iter().filter(|d| d.vertical == vertical && d.id != *id).map(|d| d.pos));
        pts
    }

    /// Moves divider `id` towards `target` (unit coordinates). `extent_px` is the pane area's size along that axis; the panes on
    /// both sides keep at least `min_px`, and the divider jumps to a snap point within `snap_px`. Returns the snap point used, if any.
    pub fn drag(&mut self, id: &DividerId, target: f32, extent_px: f32, min_px: f32, snap_px: f32) -> Option<f32> {
        let Some(Node::Split { cols, .. }) = node_at(&self.root, &id.path) else { return None };
        let vertical = *cols; let reg = self.region(&id.path);
        let pts = self.snap_points(id, vertical);
        let (start, len) = if vertical { (reg.x, reg.w) } else { (reg.y, reg.h) };
        let ext = extent_px.max(1.0);
        let thr = snap_px / ext;
        let snapped = pts.into_iter().filter(|p| (p - target).abs() <= thr).min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()));
        let t = snapped.unwrap_or(target);
        let Some(Node::Split { fracs, .. }) = node_at_mut(&mut self.root, &id.path) else { return None };
        if id.index + 1 >= fracs.len() || len <= 0.0 { return None; }
        let kid_start = start + fracs[..id.index].iter().sum::<f32>() * len;
        let pair = fracs[id.index] + fracs[id.index + 1];
        let min = (min_px / (ext * len)).min(pair / 2.0);
        let a = ((t - kid_start) / len).clamp(min, pair - min);
        fracs[id.index] = a; fracs[id.index + 1] = pair - a;
        // report a snap only when the divider really landed on it (clamping can override it)
        snapped.filter(|s| (kid_start + a * len - s).abs() < 1e-3)
    }

    /// Drags a corner: both of its dividers at once.
    pub fn drag_corner(&mut self, c: &Corner, x: f32, y: f32, size_px: (f32, f32), min_px: f32, snap_px: f32) {
        self.drag(&c.v, x, size_px.0, min_px, snap_px); self.drag(&c.h, y, size_px.1, min_px, snap_px);
    }

    /// Exchanges the places of panes `a` and `b` (sizes of the places stay); false when either is not in the layout.
    pub fn swap(&mut self, a: usize, b: usize) -> bool {
        fn go(n: &mut Node, a: usize, b: usize) { match n { Node::Leaf(i) => { if *i == a { *i = b } else if *i == b { *i = a } } Node::Split { kids, .. } => kids.iter_mut().for_each(|k| go(k, a, b)) } }
        if a == b || a >= self.n || b >= self.n { return false; }
        go(&mut self.root, a, b); true
    }
    /// Moves pane `from` next to pane `to` on `edge`: `to` is split in two and the panes around `from`'s old place grow into it.
    pub fn move_pane(&mut self, from: usize, to: usize, edge: Edge) -> bool {
        fn remove(n: &mut Node, a: usize) -> bool {
            let Node::Split { fracs, kids, .. } = n else { return false };
            if let Some(i) = kids.iter().position(|k| *k == Node::Leaf(a)) {
                kids.remove(i); fracs.remove(i); let sum: f32 = fracs.iter().sum(); fracs.iter_mut().for_each(|f| *f /= sum);
                if kids.len() == 1 { *n = kids.pop().unwrap(); }
                return true;
            }
            kids.iter_mut().any(|k| remove(k, a))
        }
        fn insert(n: &mut Node, to: usize, from: usize, edge: Edge) -> bool {
            match n {
                Node::Leaf(i) if *i == to => {
                    let cols = matches!(edge, Edge::Left | Edge::Right);
                    let kids = if matches!(edge, Edge::Left | Edge::Top) { vec![Node::Leaf(from), Node::Leaf(to)] } else { vec![Node::Leaf(to), Node::Leaf(from)] };
                    *n = Node::Split { cols, fracs: vec![0.5, 0.5], kids }; true
                }
                Node::Leaf(_) => false,
                Node::Split { kids, .. } => kids.iter_mut().any(|k| insert(k, to, from, edge)),
            }
        }
        // a split inside a split of the same direction is just more siblings
        fn flatten(n: &mut Node) {
            let Node::Split { cols, fracs, kids } = n else { return };
            kids.iter_mut().for_each(flatten);
            let (mut nf, mut nk) = (vec![], vec![]);
            for (f, k) in fracs.iter().zip(kids.drain(..)) {
                match k { Node::Split { cols: c2, fracs: f2, kids: k2 } if c2 == *cols => { for (a, b) in f2.into_iter().zip(k2) { nf.push(f * a); nk.push(b); } } k => { nf.push(*f); nk.push(k); } }
            }
            *fracs = nf; *kids = nk;
        }
        if from == to || from >= self.n || to >= self.n || !remove(&mut self.root, from) { return false; }
        let ok = insert(&mut self.root, to, from, edge); flatten(&mut self.root);
        self.custom = true; ok
    }
    /// Equal sizes again for the split containing `id` (double-click on a divider).
    pub fn reset(&mut self, id: &DividerId) {
        if let Some(Node::Split { fracs, .. }) = node_at_mut(&mut self.root, &id.path) { let n = fracs.len(); *fracs = equal(n); }
    }
    /// Equal sizes everywhere.
    pub fn reset_all(&mut self) { *self = Layout::new(self.n, self.arrangement); }
}

#[cfg(test)]
mod tests { use super::*;
    fn area(l: &Layout) -> f32 { l.rects().iter().map(|(_, r)| r.w * r.h).sum() }

    #[test] fn panes_tile_the_area() {
        for n in 1..=9 { for a in Arrangement::ALL { let l = Layout::new(n, a); assert_eq!(l.rects().len(), n); assert!((area(&l) - 1.0).abs() < 1e-4, "{n} {a:?}"); } }
    }
    #[test] fn automatic_shapes() {
        let two = Layout::new(2, Arrangement::Auto).rects(); assert_eq!((two[0].1.w, two[0].1.h), (0.5, 1.0));
        let three = Layout::new(3, Arrangement::Auto).rects(); // two on top, one wide below
        assert_eq!((three[0].1.w, three[0].1.h), (0.5, 0.5)); assert_eq!((three[2].1.w, three[2].1.h, three[2].1.y), (1.0, 0.5, 0.5));
        let four = Layout::new(4, Arrangement::Auto).rects(); assert!(four.iter().all(|(_, r)| r.w == 0.5 && r.h == 0.5));
        let rows = Layout::new(3, Arrangement::Rows).rects(); assert!(rows.iter().all(|(_, r)| r.w == 1.0));
    }
    #[test] fn drag_clamps_and_keeps_the_pair_total() {
        let mut l = Layout::new(3, Arrangement::Columns); let d = l.dividers()[0].id.clone();
        l.drag(&d, 0.01, 1000.0, 100.0, 0.0); let r = l.rects(); assert!((r[0].1.w - 0.1).abs() < 1e-4); assert!((area(&l) - 1.0).abs() < 1e-4);
        l.drag(&d, 5.0, 1000.0, 100.0, 0.0); let r = l.rects(); assert!((r[0].1.w - (2.0 / 3.0 - 0.1)).abs() < 1e-4, "stops at the next pane's minimum");
    }
    #[test] fn snaps_to_halves_thirds_and_quarters() {
        let mut l = Layout::new(2, Arrangement::Columns); let d = l.dividers()[0].id.clone();
        assert_eq!(l.drag(&d, 0.52, 1000.0, 50.0, 20.0), Some(0.5)); assert_eq!(l.rects()[0].1.w, 0.5);
        assert!(l.drag(&d, 0.6, 1000.0, 50.0, 20.0).is_none()); assert!((l.rects()[0].1.w - 0.6).abs() < 1e-5);
        assert!(l.drag(&d, 0.34, 1000.0, 50.0, 20.0).is_some()); assert!((l.rects()[0].1.w - 1.0 / 3.0).abs() < 1e-5);
        l.drag(&d, 0.74, 1000.0, 50.0, 20.0); assert!((l.rects()[0].1.w - 0.75).abs() < 1e-5);
    }
    #[test] fn snaps_to_a_divider_in_another_row() {
        // 3 panes: two on top (split at 0.5 by default) and one below; move the top divider off centre, then the grid rows stay independent
        let mut l = Layout::new(5, Arrangement::Grid); // rows of [0,1,2] [3,4]
        let ds = l.dividers(); let top = ds.iter().find(|d| d.vertical && d.id.path == vec![0] && d.id.index == 0).unwrap().id.clone();
        let bottom = ds.iter().find(|d| d.vertical && d.id.path == vec![1]).unwrap().id.clone();
        l.drag(&top, 0.3, 1000.0, 50.0, 0.0); // top row's first divider at 0.3 (not a fraction we snap to by itself... 0.3 is not a snap point)
        // dragging the bottom divider near 0.3 aligns with the one above
        assert!(l.drag(&bottom, 0.31, 1000.0, 50.0, 20.0).is_some()); let r = l.rects(); assert!((r[3].1.w - 0.3).abs() < 1e-3, "{r:?}");
    }
    #[test] fn corners_join_vertical_and_horizontal_dividers() {
        let l = Layout::new(3, Arrangement::Auto); let cs = l.corners();
        assert_eq!(cs.len(), 1); assert!((cs[0].x - 0.5).abs() < 1e-4 && (cs[0].y - 0.5).abs() < 1e-4);
        let mut l = l; let c = cs[0].clone(); l.drag_corner(&c, 0.34, 0.76, (1000.0, 800.0), 50.0, 20.0);
        let r = l.rects(); assert!((r[0].1.w - 1.0 / 3.0).abs() < 1e-4); assert!((r[0].1.h - 0.75).abs() < 1e-4);
        assert!(Layout::new(2, Arrangement::Columns).corners().is_empty());
    }
    #[test] fn reset_restores_equal_sizes() {
        let mut l = Layout::new(3, Arrangement::Rows); let d = l.dividers()[0].id.clone();
        l.drag(&d, 0.2, 1000.0, 50.0, 0.0); l.reset(&d); assert!(l.rects().iter().all(|(_, r)| (r.h - 1.0 / 3.0).abs() < 1e-5));
        l.drag(&d, 0.2, 1000.0, 50.0, 0.0); l.reset_all(); assert!(l.fits(3, Arrangement::Rows));
    }
    #[test] fn swapping_exchanges_places() {
        let mut l = Layout::new(3, Arrangement::Auto); let before = l.rects();
        assert!(l.swap(0, 2)); let after = l.rects();
        assert_eq!(after[0].1, before[2].1); assert_eq!(after[2].1, before[0].1); assert_eq!(after[1].1, before[1].1);
        assert!(!l.swap(1, 1) && !l.swap(0, 9)); assert!((area(&l) - 1.0).abs() < 1e-4);
    }
    #[test] fn moving_a_pane_splits_the_target_and_the_rest_fill_in() {
        // 3 panes: 0 and 1 on top, 2 below. Drop 2 on the left of 0: three columns, 2 | 0 | 1.
        let mut l = Layout::new(3, Arrangement::Auto); assert!(l.move_pane(2, 0, Edge::Left)); assert!(l.custom);
        let r = l.rects(); assert!(r[2].1.x < r[0].1.x && r[0].1.x < r[1].1.x);
        assert!((r[2].1.w - 0.25).abs() < 1e-4 && (r[0].1.w - 0.25).abs() < 1e-4 && (r[1].1.w - 0.5).abs() < 1e-4, "{r:?}");
        assert!(r.iter().all(|(_, x)| (x.h - 1.0).abs() < 1e-4)); assert!((area(&l) - 1.0).abs() < 1e-4);
        // 2x2 grid: drop 3 on the top of 0: 3 above 0 in the left column, 1 and 2 keep their halves
        let mut g = Layout::new(4, Arrangement::Grid); assert!(g.move_pane(3, 0, Edge::Top));
        let r = g.rects(); assert!(r[3].1.y < r[0].1.y && r[3].1.x == r[0].1.x && (r[3].1.h - 0.25).abs() < 1e-4, "{r:?}"); assert!((area(&g) - 1.0).abs() < 1e-4);
        assert!(!g.move_pane(1, 1, Edge::Left) && !g.move_pane(9, 0, Edge::Left));
        for n in 2..=7 { for a in Arrangement::ALL { for e in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] { let mut l = Layout::new(n, a); assert!(l.move_pane(n - 1, 0, e)); assert_eq!(l.rects().len(), n); assert!((area(&l) - 1.0).abs() < 1e-3, "{n} {a:?} {e:?}"); } } }
        assert!(g.fits(4, Arrangement::Rows) && !g.fits(3, Arrangement::Grid), "custom layouts survive an arrangement change until reset");
    }
    #[test] fn drop_zones() {
        assert_eq!(drop_zone(0.1, 0.5), Drop::Split(Edge::Left)); assert_eq!(drop_zone(0.9, 0.5), Drop::Split(Edge::Right));
        assert_eq!(drop_zone(0.5, 0.05), Drop::Split(Edge::Top)); assert_eq!(drop_zone(0.5, 0.95), Drop::Split(Edge::Bottom)); assert_eq!(drop_zone(0.5, 0.5), Drop::Swap);
    }
    #[test] fn arrangement_serde() { #[derive(Serialize, Deserialize)] struct T { a: Arrangement, b: BatchView } let t: T = toml::from_str("a = \"grid\"\nb = \"stacked\"").unwrap(); assert_eq!((t.a, t.b), (Arrangement::Grid, BatchView::Stacked)); }
}
