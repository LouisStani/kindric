//! The panes of the window: a tree of splits whose leaves show at most one session each.
//! The page only draws it; every change goes through here.

use serde::{Deserialize, Serialize};

pub type PaneId = u64;

/// A dragged separator never hides a pane completely.
const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 0.9;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionKey {
    pub account: String,
    pub short_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Side by side.
    Row,
    /// One above the other.
    Column,
}

/// Where a new pane goes in the pane it cuts: before is left or above, after is right or below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Before,
    After,
}

/// Side by side unless the pane is taller than wide.
fn direction_for(width: f64, height: f64) -> Direction {
    if width >= height { Direction::Row } else { Direction::Column }
}

/// The side of a new pane that puts `pointer`, measured from the top left corner of the pane cut,
/// inside it: the cut is in the middle. After when the pointer is not over that pane.
pub fn side_under(width: f64, height: f64, pointer: Option<(f64, f64)>) -> Side {
    let Some((x, y)) = pointer.filter(|&(x, y)| (0.0..width).contains(&x) && (0.0..height).contains(&y)) else {
        return Side::After;
    };
    let before = match direction_for(width, height) {
        Direction::Row => x < width / 2.0,
        Direction::Column => y < height / 2.0,
    };
    if before { Side::Before } else { Side::After }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Node {
    Leaf {
        id: PaneId,
        session: Option<SessionKey>,
    },
    Split {
        id: PaneId,
        direction: Direction,
        /// Share of the first child.
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Shown {
    /// Not shown anywhere: it must be attached in the pane asked for.
    Attach,
    /// Already shown in this pane, which now has the focus.
    Already(PaneId),
}

/// Hands out the ids of panes and splits. One counter serves every tab, so a pane id is
/// unique in the whole window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ids {
    next: PaneId,
}

impl Default for Ids {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl Ids {
    pub fn take(&mut self) -> PaneId {
        let id = self.next;
        self.next += 1;
        id
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Layout {
    root: Node,
    active: PaneId,
}

fn collect_leaves<'a>(node: &'a Node, found: &mut Vec<(PaneId, &'a Option<SessionKey>)>) {
    match node {
        Node::Leaf { id, session } => found.push((*id, session)),
        Node::Split { first, second, .. } => {
            collect_leaves(first, found);
            collect_leaves(second, found);
        },
    }
}

fn find_mut(node: &mut Node, wanted: PaneId) -> Option<&mut Node> {
    match node {
        Node::Leaf { id, .. } if *id == wanted => Some(node),
        Node::Split { id, .. } if *id == wanted => Some(node),
        Node::Split { first, second, .. } => find_mut(first, wanted).or_else(|| find_mut(second, wanted)),
        Node::Leaf { .. } => None,
    }
}

fn is_leaf(node: &Node, wanted: PaneId) -> bool {
    matches!(node, Node::Leaf { id, .. } if *id == wanted)
}

/// The tree without the leaf `wanted`: its sibling takes the place of their parent.
fn without(node: Node, wanted: PaneId) -> Node {
    match node {
        Node::Split { first, second, .. } if is_leaf(&first, wanted) => *second,
        Node::Split { first, second, .. } if is_leaf(&second, wanted) => *first,
        Node::Split { id, direction, ratio, first, second } => Node::Split {
            id,
            direction,
            ratio,
            first: Box::new(without(*first, wanted)),
            second: Box::new(without(*second, wanted)),
        },
        leaf => leaf,
    }
}

impl Layout {
    /// One empty pane, active.
    pub fn new(ids: &mut Ids) -> Self {
        let pane = ids.take();
        Self { root: Node::Leaf { id: pane, session: None }, active: pane }
    }

    #[cfg(test)]
    pub fn active(&self) -> PaneId {
        self.active
    }

    /// Panes in reading order (left to right, top to bottom): the order of Ctrl+H/L.
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut found = Vec::new();
        collect_leaves(&self.root, &mut found);
        found.into_iter().map(|(id, _)| id).collect()
    }

    /// Cuts the active pane in two, side by side unless it is taller than wide. The new,
    /// empty pane goes on `side` and gets the focus.
    pub fn split(&mut self, ids: &mut Ids, width: f64, height: f64, side: Side) -> PaneId {
        let direction = direction_for(width, height);
        let split_id = ids.take();
        let pane = ids.take();
        let active = self.active;
        if let Some(node) = find_mut(&mut self.root, active) {
            let old = Box::new(std::mem::replace(node, Node::Leaf { id: 0, session: None }));
            let new = Box::new(Node::Leaf { id: pane, session: None });
            let (first, second) = match side {
                Side::Before => (new, old),
                Side::After => (old, new),
            };
            *node = Node::Split { id: split_id, direction, ratio: 0.5, first, second };
            self.active = pane;
        }
        pane
    }

    /// Closes the active pane and gives the focus to the one before it. The last pane
    /// stays: returns the closed pane, or None.
    pub fn close(&mut self) -> Option<PaneId> {
        let order = self.leaves();
        if order.len() < 2 {
            return None;
        }
        let closed = self.active;
        let index = order.iter().position(|id| *id == closed)?;
        let root = std::mem::replace(&mut self.root, Node::Leaf { id: 0, session: None });
        self.root = without(root, closed);
        self.active = if index > 0 { order[index - 1] } else { order[1] };
        Some(closed)
    }

    pub fn focus(&mut self, pane: PaneId) -> bool {
        let known = self.leaves().contains(&pane);
        if known {
            self.active = pane;
        }
        known
    }

    /// Moves the focus `step` panes forward (or back if negative), wrapping around.
    pub fn focus_step(&mut self, step: i64) {
        let order = self.leaves();
        let Some(index) = order.iter().position(|id| *id == self.active) else {
            return;
        };
        let length = order.len() as i64;
        self.active = order[(index as i64 + step).rem_euclid(length) as usize];
    }

    pub fn set_ratio(&mut self, split: PaneId, ratio: f32) {
        if !ratio.is_finite() {
            return;
        }
        if let Some(Node::Split { ratio: current, .. }) = find_mut(&mut self.root, split) {
            *current = ratio.clamp(MIN_RATIO, MAX_RATIO);
        }
    }

    /// The session in the active pane, if any.
    pub fn active_session(&self) -> Option<&SessionKey> {
        let mut found = Vec::new();
        collect_leaves(&self.root, &mut found);
        found.into_iter().find(|(id, _)| *id == self.active).and_then(|(_, session)| session.as_ref())
    }

    /// The pane showing `session`, if any.
    pub fn shown_in(&self, session: &SessionKey) -> Option<PaneId> {
        let mut found = Vec::new();
        collect_leaves(&self.root, &mut found);
        found.into_iter().find(|(_, key)| key.as_ref() == Some(session)).map(|(id, _)| id)
    }

    /// Where `session` should appear when it is opened in `pane`; None if the pane is unknown.
    pub fn show(&mut self, pane: PaneId, session: SessionKey) -> Option<Shown> {
        if let Some(shown_in) = self.shown_in(&session) {
            self.active = shown_in;
            return Some(Shown::Already(shown_in));
        }
        let Some(Node::Leaf { session: slot, .. }) = find_mut(&mut self.root, pane) else {
            return None;
        };
        *slot = Some(session);
        self.active = pane;
        Some(Shown::Attach)
    }

    /// The pane no longer shows a session (it ended, or could not be attached).
    pub fn clear(&mut self, pane: PaneId) {
        if let Some(Node::Leaf { session, .. }) = find_mut(&mut self.root, pane) {
            *session = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(short_id: &str) -> SessionKey {
        SessionKey { account: "lystriq".into(), short_id: short_id.into() }
    }

    fn direction_of_root(layout: &Layout) -> Option<Direction> {
        match &layout.root {
            Node::Split { direction, .. } => Some(*direction),
            Node::Leaf { .. } => None,
        }
    }

    fn fresh() -> (Layout, Ids) {
        let mut ids = Ids::default();
        (Layout::new(&mut ids), ids)
    }

    /// Four panes: 1 | (3 above (5 | 7)).
    fn four_panes() -> Layout {
        let (mut layout, mut ids) = fresh();
        layout.split(&mut ids, 1000.0, 500.0, Side::After);
        layout.split(&mut ids, 500.0, 600.0, Side::After);
        layout.split(&mut ids, 500.0, 300.0, Side::After);
        layout
    }

    #[test]
    fn a_new_layout_has_one_empty_active_pane() {
        let (layout, _) = fresh();
        assert_eq!(layout.leaves(), [1]);
        assert_eq!(layout.active(), 1);
    }

    #[test]
    fn split_direction_follows_the_shape_of_the_pane() {
        let (mut wide, mut ids) = fresh();
        wide.split(&mut ids, 1000.0, 500.0, Side::After);
        assert_eq!(direction_of_root(&wide), Some(Direction::Row));
        let (mut tall, mut ids) = fresh();
        tall.split(&mut ids, 500.0, 1000.0, Side::After);
        assert_eq!(direction_of_root(&tall), Some(Direction::Column));
        let (mut square, mut ids) = fresh();
        square.split(&mut ids, 500.0, 500.0, Side::After);
        assert_eq!(direction_of_root(&square), Some(Direction::Row));
    }

    #[test]
    fn the_new_pane_comes_after_the_split_one_and_gets_the_focus() {
        let (mut layout, mut ids) = fresh();
        let pane = layout.split(&mut ids, 1000.0, 500.0, Side::After);
        assert_eq!(layout.leaves(), [1, pane]);
        assert_eq!(layout.active(), pane);
        assert_eq!(four_panes().leaves(), [1, 3, 5, 7]);
    }

    #[test]
    fn the_new_pane_goes_where_the_pointer_is() {
        assert_eq!(side_under(1000.0, 500.0, Some((100.0, 400.0))), Side::Before);
        assert_eq!(side_under(1000.0, 500.0, Some((900.0, 100.0))), Side::After);
        assert_eq!(side_under(500.0, 1000.0, Some((400.0, 100.0))), Side::Before);
        assert_eq!(side_under(500.0, 1000.0, Some((100.0, 900.0))), Side::After);
    }

    #[test]
    fn without_the_pointer_over_the_pane_the_new_pane_goes_after() {
        assert_eq!(side_under(1000.0, 500.0, None), Side::After);
        assert_eq!(side_under(1000.0, 500.0, Some((-10.0, 100.0))), Side::After);
        assert_eq!(side_under(1000.0, 500.0, Some((100.0, 500.0))), Side::After);
    }

    #[test]
    fn a_new_pane_before_comes_first_and_gets_the_focus() {
        let (mut layout, mut ids) = fresh();
        let pane = layout.split(&mut ids, 1000.0, 500.0, Side::Before);
        assert_eq!(layout.leaves(), [pane, 1]);
        assert_eq!(layout.active(), pane);
        let above = layout.split(&mut ids, 500.0, 1000.0, Side::Before);
        assert_eq!(direction_of_root(&layout), Some(Direction::Row));
        assert_eq!(layout.leaves(), [above, pane, 1]);
    }

    #[test]
    fn closing_a_middle_pane_lifts_its_sibling_and_focuses_the_previous_pane() {
        let mut layout = four_panes();
        layout.focus(5);
        assert_eq!(layout.close(), Some(5));
        assert_eq!(layout.leaves(), [1, 3, 7]);
        assert_eq!(layout.active(), 3);
        let Node::Split { second, .. } = &layout.root else { panic!("root should stay a split") };
        let Node::Split { direction, first, second: lifted, .. } = second.as_ref() else {
            panic!("3 and 7 should share a split")
        };
        assert_eq!(*direction, Direction::Column);
        assert!(is_leaf(first, 3));
        assert!(is_leaf(lifted, 7));
    }

    #[test]
    fn closing_the_first_pane_focuses_the_next_one() {
        let mut layout = four_panes();
        layout.focus(1);
        assert_eq!(layout.close(), Some(1));
        assert_eq!(layout.leaves(), [3, 5, 7]);
        assert_eq!(layout.active(), 3);
    }

    #[test]
    fn the_last_pane_cannot_be_closed() {
        let (mut layout, mut ids) = fresh();
        assert_eq!(layout.close(), None);
        assert_eq!(layout.leaves(), [1]);
        layout.split(&mut ids, 1000.0, 500.0, Side::After);
        assert!(layout.close().is_some());
        assert_eq!(layout.close(), None);
        assert_eq!(layout.leaves(), [1]);
    }

    #[test]
    fn focus_steps_wrap_around_in_reading_order() {
        let mut layout = four_panes();
        assert_eq!(layout.active(), 7);
        layout.focus_step(1);
        assert_eq!(layout.active(), 1);
        layout.focus_step(-1);
        assert_eq!(layout.active(), 7);
        layout.focus_step(-1);
        assert_eq!(layout.active(), 5);
    }

    #[test]
    fn focus_refuses_unknown_panes_and_splits() {
        let mut layout = four_panes();
        assert!(layout.focus(3));
        assert!(!layout.focus(2), "2 is a split, not a pane");
        assert!(!layout.focus(99));
        assert_eq!(layout.active(), 3);
    }

    #[test]
    fn ratio_is_bounded_and_unknown_splits_are_ignored() {
        let (mut layout, mut ids) = fresh();
        layout.split(&mut ids, 1000.0, 500.0, Side::After);
        let ratio = |layout: &Layout| match &layout.root {
            Node::Split { ratio, .. } => *ratio,
            Node::Leaf { .. } => f32::NAN,
        };
        layout.set_ratio(2, 0.3);
        assert_eq!(ratio(&layout), 0.3);
        layout.set_ratio(2, 0.0);
        assert_eq!(ratio(&layout), MIN_RATIO);
        layout.set_ratio(2, 2.0);
        assert_eq!(ratio(&layout), MAX_RATIO);
        layout.set_ratio(2, f32::NAN);
        assert_eq!(ratio(&layout), MAX_RATIO);
        let before = layout.clone();
        layout.set_ratio(99, 0.3);
        layout.set_ratio(1, 0.3);
        assert_eq!(layout, before);
    }

    #[test]
    fn a_session_shown_elsewhere_moves_the_focus_instead_of_attaching_twice() {
        let (mut layout, mut ids) = fresh();
        assert_eq!(layout.show(1, key("aaa")), Some(Shown::Attach));
        let second = layout.split(&mut ids, 1000.0, 500.0, Side::After);
        assert_eq!(layout.show(second, key("aaa")), Some(Shown::Already(1)));
        assert_eq!(layout.active(), 1);
        assert_eq!(layout.show(second, key("bbb")), Some(Shown::Attach));
        assert_eq!(layout.active(), second);
        assert_eq!(layout.show(99, key("ccc")), None);
    }

    #[test]
    fn a_cleared_pane_can_show_its_session_again() {
        let (mut layout, _) = fresh();
        layout.show(1, key("aaa"));
        layout.clear(1);
        assert_eq!(layout.show(1, key("aaa")), Some(Shown::Attach));
    }

    #[test]
    fn the_page_gets_the_tree_and_the_active_pane() {
        let (mut layout, mut ids) = fresh();
        layout.split(&mut ids, 1000.0, 500.0, Side::After);
        layout.show(3, key("aaa"));
        let json = serde_json::to_value(&layout).unwrap_or_default();
        assert_eq!(json["active"], 3);
        assert_eq!(json["root"]["kind"], "split");
        assert_eq!(json["root"]["direction"], "row");
        assert_eq!(json["root"]["second"]["session"]["short_id"], "aaa");
    }

    #[test]
    fn the_active_session_follows_the_focus() {
        let (mut layout, mut ids) = fresh();
        assert_eq!(layout.active_session(), None);
        layout.show(1, key("aaa"));
        assert_eq!(layout.active_session(), Some(&key("aaa")));
        layout.split(&mut ids, 1000.0, 500.0, Side::After);
        assert_eq!(layout.active_session(), None, "the new pane is empty");
        layout.focus(1);
        assert_eq!(layout.active_session(), Some(&key("aaa")));
    }
}
