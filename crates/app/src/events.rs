//! Detects what changed about a merge request between two polls, as user-facing events.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot { pub state: String, pub head: String, pub ci: Option<String> }

#[derive(Debug, Clone, PartialEq)]
pub struct Event { pub title: String, pub body: String, /// also worth a desktop notification
    pub important: bool, pub level: Level }
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level { Info, Success, Warning, Error }

/// `is_ancestor(old, new)` says whether the previous head is still in the new head's history (false = force-push).
pub fn diff(prev: &Snapshot, now: &Snapshot, title: &str, is_ancestor: &dyn Fn(&str, &str) -> bool) -> Vec<Event> {
    let mut out = vec![];
    let ev = |t: &str, b: String, imp: bool, l: Level| Event { title: t.into(), body: b, important: imp, level: l };
    if prev.state != now.state && !prev.state.is_empty() {
        match now.state.as_str() {
            "merged" => out.push(ev("Merged", format!("{title} was merged."), true, Level::Success)),
            "closed" => out.push(ev("Closed", format!("{title} was closed without merging."), true, Level::Warning)),
            "open" => out.push(ev("Reopened", format!("{title} was reopened."), true, Level::Info)),
            _ => {}
        }
    }
    if !prev.head.is_empty() && prev.head != now.head && now.state == "open" {
        if is_ancestor(&prev.head, &now.head) { out.push(ev("New commits", format!("New commits were pushed to {title}. Move your review to the new head to see them."), true, Level::Info)); }
        else { out.push(ev("History rewritten", format!("{title} was force-pushed. Your drafts are kept; move to the new head to re-anchor them."), true, Level::Warning)); }
    }
    if prev.ci != now.ci && prev.ci.is_some() || (prev.ci.is_none() && now.ci.as_deref().map(|c| c == "failed").unwrap_or(false) && !prev.head.is_empty()) {
        match now.ci.as_deref() {
            Some("passed") => out.push(ev("Checks passed", format!("CI passed for {title}."), true, Level::Success)),
            Some("failed") => out.push(ev("Checks failed", format!("CI failed for {title}."), true, Level::Error)),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests { use super::*;
    fn s(state: &str, head: &str, ci: Option<&str>) -> Snapshot { Snapshot { state: state.into(), head: head.into(), ci: ci.map(String::from) } }
    #[test] fn detects_merge_close_push_force_and_ci() {
        let anc = |a: &str, _: &str| a == "old";
        assert!(diff(&s("open", "a", None), &s("open", "a", None), "T", &anc).is_empty());
        assert_eq!(diff(&s("open", "a", None), &s("merged", "a", None), "T", &anc)[0].title, "Merged");
        assert_eq!(diff(&s("open", "a", None), &s("closed", "a", None), "T", &anc)[0].level, Level::Warning);
        assert_eq!(diff(&s("open", "old", None), &s("open", "new", None), "T", &anc)[0].title, "New commits");
        assert_eq!(diff(&s("open", "x", None), &s("open", "new", None), "T", &anc)[0].title, "History rewritten");
        assert_eq!(diff(&s("open", "a", Some("running")), &s("open", "a", Some("failed")), "T", &anc)[0].title, "Checks failed");
        assert_eq!(diff(&s("open", "a", Some("running")), &s("open", "a", Some("passed")), "T", &anc)[0].level, Level::Success);
        assert!(diff(&s("", "", None), &s("open", "a", None), "T", &anc).is_empty()); // first poll is not a change
        assert!(diff(&s("open", "a", Some("running")), &s("open", "a", Some("running")), "T", &anc).is_empty());
    } }
