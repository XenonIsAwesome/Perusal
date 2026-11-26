//! Free-text search for the Home lists (recent reviews, my reviews).

/// True when every whitespace-separated term of `query` occurs (case-insensitively) in at least one of `fields`.
/// An empty query matches everything.
pub fn matches(query: &str, fields: &[&str]) -> bool {
    let hay: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    query.split_whitespace().all(|t| { let t = t.to_lowercase(); hay.iter().any(|h| h.contains(&t)) })
}

/// What the My reviews search sees of one request.
pub struct Item<'a> { pub title: &'a str, pub host: &'a str, pub project: &'a str, pub number: u64, pub author: &'a str, pub state: &'a str, pub kind: &'a str,
    pub assignees: Vec<&'a str>, pub reviewers: Vec<&'a str>, pub labels: Vec<&'a str>, pub description: &'a str, pub draft: bool }

/// Search with qualifiers. `author:bob`, `assignee:me-or-name`, `reviewer:x`, `label:bug`, `project:grp/app`, `host:corp`, `state:open`,
/// `kind:github` (or `gitlab`), `is:draft`, `-is:draft`, `in:description`-style text always also searches the overview text.
/// A qualifier matches by substring (case-insensitive); a leading `-` negates it. Other terms must all occur in the title, host, project,
/// number, author, assignees, reviewers, labels, state or overview text.
pub fn matches_item(query: &str, it: &Item) -> bool {
    let low = |v: &str| v.to_lowercase();
    let any = |vals: &[&str], t: &str| vals.iter().any(|v| low(v).contains(t));
    let hay: Vec<String> = [it.title, it.host, it.project, it.author, it.state, it.kind, it.description].iter().map(|v| low(v)).chain(std::iter::once(it.number.to_string()))
        .chain(it.assignees.iter().chain(&it.reviewers).chain(&it.labels).map(|v| low(v))).collect();
    query.split_whitespace().all(|raw| {
        let (neg, raw) = match raw.strip_prefix('-') { Some(r) if r.contains(':') => (true, r), _ => (false, raw) };
        let t = low(raw);
        let hit = match t.split_once(':') {
            Some((k, v)) if !v.is_empty() => match k {
                "author" | "by" => low(it.author).contains(v), "assignee" | "assigned" => any(&it.assignees, v), "reviewer" | "review" => any(&it.reviewers, v),
                "label" => any(&it.labels, v), "project" | "repo" => low(it.project).contains(v), "host" => low(it.host).contains(v),
                "state" => low(it.state) == v || low(it.state).starts_with(v), "kind" | "type" => low(it.kind) == v,
                "is" => match v { "draft" => it.draft, "open" | "merged" | "closed" => low(it.state) == v, _ => false },
                "user" | "person" | "who" => low(it.author).contains(v) || any(&it.assignees, v) || any(&it.reviewers, v),
                _ => hay.iter().any(|h| h.contains(&t)),
            },
            _ => hay.iter().any(|h| h.contains(&t)),
        };
        hit != neg
    })
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn terms_are_anded_and_case_insensitive() {
        let f = ["Fix login bug", "gitlab.corp", "grp/proj", "bob"];
        assert!(matches("", &f) && matches("   ", &f));
        assert!(matches("LOGIN", &f) && matches("login bob", &f) && matches("proj corp", &f));
        assert!(!matches("login alice", &f) && !matches("zzz", &f));
    }
    fn item<'a>() -> Item<'a> { Item { title: "Fix login", host: "gitlab.corp", project: "grp/app", number: 7, author: "bob", state: "open", kind: "gitlab",
        assignees: vec!["alice"], reviewers: vec!["carol"], labels: vec!["bug", "p1"], description: "Rewrites the session cache", draft: true } }
    #[test] fn qualifiers_and_overview_text() {
        let i = item();
        assert!(matches_item("", &i) && matches_item("author:BOB assignee:ali reviewer:car label:bug", &i) && matches_item("cache", &i) && matches_item("is:draft kind:gitlab state:open", &i));
        assert!(!matches_item("author:alice", &i) && !matches_item("kind:github", &i) && !matches_item("label:feature", &i));
        assert!(matches_item("-label:feature -is:merged", &i) && !matches_item("-author:bob", &i) && matches_item("user:carol", &i));
    }
}
