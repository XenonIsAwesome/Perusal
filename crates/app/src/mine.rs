//! My reviews filtering: multi-select author / assignee / label filters (OR within one filter, AND between filters),
//! the Remote chip, the search box, and the options each dropdown offers.
use crate::search::{matches_item, Item};
use std::collections::BTreeSet;

/// Stands for the signed-in user (a different login per host) in the people filters.
pub const ME: &str = "@me";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim { Author, Assignee, Label }

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sel { pub authors: BTreeSet<String>, pub assignees: BTreeSet<String>, pub labels: BTreeSet<String> }
impl Sel {
    pub fn set(&self, d: Dim) -> &BTreeSet<String> { match d { Dim::Author => &self.authors, Dim::Assignee => &self.assignees, Dim::Label => &self.labels } }
    pub fn set_mut(&mut self, d: Dim) -> &mut BTreeSet<String> { match d { Dim::Author => &mut self.authors, Dim::Assignee => &mut self.assignees, Dim::Label => &mut self.labels } }
    pub fn toggle(&mut self, d: Dim, v: &str) { let s = self.set_mut(d); if !s.remove(v) { s.insert(v.to_string()); } }
    pub fn is_empty(&self) -> bool { self.authors.is_empty() && self.assignees.is_empty() && self.labels.is_empty() }
}

fn person_ok(sel: &BTreeSet<String>, me: Option<&str>, people: &[&str]) -> bool {
    sel.is_empty() || sel.iter().any(|w| { let w = if w == ME { match me { Some(m) => m, None => return false } } else { w.as_str() }; people.iter().any(|p| p.eq_ignore_ascii_case(w)) })
}
fn label_ok(sel: &BTreeSet<String>, labels: &[&str]) -> bool { sel.is_empty() || sel.iter().any(|w| labels.iter().any(|l| l == w)) }

/// Does `it` pass every active filter? `skip` leaves one dropdown's own selection out (used to list what it could still add).
/// `remote` is `Some("github")` / `Some("gitlab")` for the Remote chip.
pub fn passes(it: &Item, remote: Option<&str>, sel: &Sel, query: &str, skip: Option<Dim>) -> bool {
    remote.map(|r| r == it.kind).unwrap_or(true)
        && (skip == Some(Dim::Author) || person_ok(&sel.authors, it.me, &[it.author]))
        && (skip == Some(Dim::Assignee) || person_ok(&sel.assignees, it.me, &it.assignees))
        && (skip == Some(Dim::Label) || label_ok(&sel.labels, &it.labels))
        && matches_item(query, it)
}

/// Values a dropdown offers: only those present on the requests that pass all the *other* filters, most frequent first, plus anything
/// already selected (so it can be un-picked). `@me` is never listed; the UI puts "Me" first itself.
pub fn options(items: &[Item], d: Dim, remote: Option<&str>, sel: &Sel, query: &str) -> Vec<String> {
    let mut count: std::collections::BTreeMap<String, usize> = Default::default();
    for it in items.iter().filter(|it| passes(it, remote, sel, query, Some(d))) {
        let vals: Vec<&str> = match d { Dim::Author => vec![it.author], Dim::Assignee => it.assignees.clone(), Dim::Label => it.labels.clone() };
        for v in vals { if !v.is_empty() { *count.entry(v.to_string()).or_default() += 1; } }
    }
    for v in sel.set(d) { if v != ME { count.entry(v.clone()).or_default(); } }
    let mut v: Vec<(String, usize)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    v.into_iter().map(|x| x.0).collect()
}

/// Trigger text of a dropdown: its name, or the single choice, or a count.
pub fn summary(name: &str, sel: &BTreeSet<String>) -> String {
    match sel.len() { 0 => name.to_string(), 1 => { let v = sel.iter().next().unwrap(); format!("{name}: {}", if v == ME { "Me" } else { v }) } n => format!("{name}: {n} selected") }
}

#[cfg(test)]
mod tests { use super::*;
    fn it<'a>(author: &'a str, assignees: Vec<&'a str>, labels: Vec<&'a str>, kind: &'a str) -> Item<'a> {
        Item { title: "t", host: "h", project: "p", number: 1, author, state: "open", kind, assignees, reviewers: vec![], labels, description: "", draft: false, me: Some("me1") }
    }
    #[test] fn or_within_and_between() {
        let items = vec![it("al", vec!["bo"], vec!["bug"], "github"), it("me1", vec!["cy"], vec!["ui"], "gitlab"), it("al", vec!["me1"], vec!["bug", "ui"], "gitlab")];
        let mut s = Sel::default();
        assert_eq!(items.iter().filter(|i| passes(i, None, &s, "", None)).count(), 3);
        s.toggle(Dim::Author, ME); s.toggle(Dim::Author, "al");
        assert_eq!(items.iter().filter(|i| passes(i, None, &s, "", None)).count(), 3);
        s.toggle(Dim::Assignee, ME);
        assert_eq!(items.iter().filter(|i| passes(i, None, &s, "", None)).count(), 1);
        assert_eq!(items.iter().filter(|i| passes(i, Some("github"), &Sel::default(), "", None)).count(), 1);
        s.toggle(Dim::Assignee, ME); s.toggle(Dim::Label, "ui"); s.toggle(Dim::Label, "bug");
        assert_eq!(items.iter().filter(|i| passes(i, None, &s, "", None)).count(), 3);
    }
    #[test] fn options_come_from_what_is_left() {
        let items = vec![it("al", vec!["bo"], vec!["bug"], "github"), it("me1", vec!["cy"], vec!["ui"], "gitlab"), it("al", vec!["bo", "cy"], vec!["bug"], "github")];
        let none = Sel::default();
        assert_eq!(options(&items, Dim::Assignee, None, &none, ""), vec!["bo", "cy"]);
        assert_eq!(options(&items, Dim::Label, Some("github"), &none, ""), vec!["bug"]);
        // authors only from items that pass the assignee filter; the author filter's own choice does not hide its siblings
        let mut s = Sel::default(); s.toggle(Dim::Assignee, "cy");
        assert_eq!(options(&items, Dim::Author, None, &s, ""), vec!["al", "me1"]);
        s.toggle(Dim::Author, "al"); assert_eq!(options(&items, Dim::Author, None, &s, ""), vec!["al", "me1"]);
        // a selected value that no longer exists stays listed so it can be removed
        let mut s = Sel::default(); s.toggle(Dim::Label, "gone"); assert!(options(&items, Dim::Label, None, &s, "").contains(&"gone".to_string()));
    }
    #[test] fn me_needs_a_known_login() {
        let mut i = it("me1", vec![], vec![], "github"); i.me = None; let mut s = Sel::default(); s.toggle(Dim::Author, ME);
        assert!(!passes(&i, None, &s, "", None));
        assert_eq!(summary("Author", &s.authors), "Author: Me"); assert_eq!(summary("Label", &Sel::default().labels), "Label");
    }
}
