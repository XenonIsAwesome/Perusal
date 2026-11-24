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
#[cfg(test)]
mod tests { use super::*;
    #[test] fn tabs() { let mut t = Tabs::default(); t.open("a"); t.open("b"); assert_eq!(t.open("a"), 0); t.next(); assert_eq!(t.active, 1); t.next(); assert_eq!(t.active, 0); t.prev(); assert_eq!(t.active, 1); t.close(1); assert_eq!(t.active, 0); } }
