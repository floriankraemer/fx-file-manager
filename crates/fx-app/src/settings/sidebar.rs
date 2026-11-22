use super::Settings;
use crate::place_group::PlaceGroup;

impl Settings {
    /// The visible sidebar groups in the saved order.
    pub fn sidebar_groups(&self) -> Vec<PlaceGroup> {
        let mut out = self.sidebar_order_full();
        out.retain(|g| self.sidebar_group_shown(*g));
        out
    }

    pub fn sidebar_group_shown(&self, group: PlaceGroup) -> bool {
        !self.hidden_sidebar_groups.contains(&group)
    }

    pub fn set_sidebar_group_shown(&mut self, group: PlaceGroup, shown: bool) {
        self.hidden_sidebar_groups.retain(|g| *g != group);
        if !shown {
            self.hidden_sidebar_groups.push(group);
        }
    }

    /// Moves `group` to the slot `target` occupies, shifting the others. Hidden groups keep their slots.
    pub fn move_sidebar_group(&mut self, group: PlaceGroup, target: PlaceGroup) {
        let mut order = self.sidebar_order_full();
        move_to_slot_of(&mut order, group, target);
        self.sidebar_order = order;
    }

    /// The saved order completed: groups missing from a hand-edited file go last, repeats are dropped.
    fn sidebar_order_full(&self) -> Vec<PlaceGroup> {
        let mut out: Vec<PlaceGroup> = Vec::new();
        for g in self.sidebar_order.iter().chain(&PlaceGroup::ALL) {
            if !out.contains(g) {
                out.push(*g);
            }
        }
        out
    }
}

/// Moves `item` to the slot `target` occupies, shifting the others.
pub(super) fn move_to_slot_of<T: PartialEq + Copy>(items: &mut Vec<T>, item: T, target: T) {
    let at = items.iter().position(|i| *i == target).unwrap_or(0);
    items.retain(|i| *i != item);
    items.insert(at.min(items.len()), item);
}
