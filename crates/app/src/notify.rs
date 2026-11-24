//! In-app notifications (toasts): one queue for every message the app shows. UI-free.
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { Info, Success, Warning, Error }

#[derive(Debug, Clone)]
pub struct Toast { pub id: u64, pub kind: Kind, pub text: String, pub created: Instant, pub ttl: Duration }

#[derive(Default)]
pub struct Toasts { items: Vec<Toast>, next: u64 }

impl Toasts {
    /// Default lifetimes: errors stay longer than confirmations.
    pub fn ttl(kind: Kind) -> Duration { Duration::from_secs(match kind { Kind::Error => 12, Kind::Warning => 9, _ => 5 }) }
    /// Adds a toast (an identical live one is refreshed instead of duplicated). Returns its id.
    pub fn push(&mut self, kind: Kind, text: impl Into<String>) -> u64 {
        let text = text.into(); let now = Instant::now();
        if let Some(t) = self.items.iter_mut().find(|t| t.text == text && t.kind == kind) { t.created = now; return t.id; }
        self.next += 1; self.items.push(Toast { id: self.next, kind, text, created: now, ttl: Self::ttl(kind) });
        if self.items.len() > 5 { self.items.remove(0); }
        self.next
    }
    pub fn dismiss(&mut self, id: u64) { self.items.retain(|t| t.id != id); }
    /// Drops expired toasts; returns true if anything changed.
    pub fn expire(&mut self, now: Instant) -> bool { let n = self.items.len(); self.items.retain(|t| now.duration_since(t.created) < t.ttl); n != self.items.len() }
    pub fn items(&self) -> &[Toast] { &self.items }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn queue_dedup_expiry_and_cap() {
        let mut t = Toasts::default(); let a = t.push(Kind::Success, "saved"); let b = t.push(Kind::Success, "saved"); assert_eq!(a, b); assert_eq!(t.items().len(), 1);
        t.push(Kind::Error, "boom"); assert!(t.items()[1].ttl > t.items()[0].ttl);
        let later = Instant::now() + Duration::from_secs(6); assert!(t.expire(later)); assert_eq!(t.items().len(), 1); assert_eq!(t.items()[0].text, "boom");
        t.dismiss(t.items()[0].id); assert!(t.is_empty());
        for i in 0..9 { t.push(Kind::Info, format!("m{i}")); } assert_eq!(t.items().len(), 5); assert_eq!(t.items()[4].text, "m8");
    } }
