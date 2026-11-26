//! Tabs in one window; opening the same MR focuses its tab.
#[derive(Default)]
pub struct Tabs { pub keys: Vec<String>, pub active: usize }
impl Tabs {
    pub fn open(&mut self, key: &str) -> usize {
        if let Some(i) = self.keys.iter().position(|k| k == key) { self.active = i; } else { self.keys.push(key.into()); self.active = self.keys.len() - 1; }
        self.active
    }
    pub fn close(&mut self, i: usize) { if i < self.keys.len() { self.keys.remove(i); if self.active >= self.keys.len() { self.active = self.keys.len().saturating_sub(1); } } }
    pub fn next(&mut self) { if !self.keys.is_empty() { self.active = (self.active + 1) % self.keys.len(); } }
    pub fn prev(&mut self) { if !self.keys.is_empty() { self.active = (self.active + self.keys.len() - 1) % self.keys.len(); } }
}
/// Where the active tab ends up after tab `from` is dragged to position `to` (the other tabs shift to make room).
pub fn reordered_active(active: Option<usize>, from: usize, to: usize) -> Option<usize> {
    active.map(|a| if a == from { to } else if from < to && a > from && a <= to { a - 1 } else if from > to && a >= to && a < from { a + 1 } else { a })
}
#[cfg(test)]
mod tests { use super::*;
    #[test] fn dragging_a_tab_keeps_the_active_one() {
        // [a b c d], move a to the end: [b c d a]
        assert_eq!(reordered_active(Some(0), 0, 3), Some(3)); assert_eq!(reordered_active(Some(1), 0, 3), Some(0)); assert_eq!(reordered_active(Some(3), 0, 3), Some(2));
        // move d to the front: [d a b c]
        assert_eq!(reordered_active(Some(3), 3, 0), Some(0)); assert_eq!(reordered_active(Some(0), 3, 0), Some(1)); assert_eq!(reordered_active(None, 3, 0), None);
    }
    #[test] fn tabs() { let mut t = Tabs::default(); t.open("a"); t.open("b"); assert_eq!(t.open("a"), 0); t.next(); assert_eq!(t.active, 1); t.next(); assert_eq!(t.active, 0); t.prev(); assert_eq!(t.active, 1); t.close(1); assert_eq!(t.active, 0); } }
