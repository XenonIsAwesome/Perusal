//! Free-text search for the Home lists (recent reviews, my reviews).

/// True when every whitespace-separated term of `query` occurs (case-insensitively) in at least one of `fields`.
/// An empty query matches everything.
pub fn matches(query: &str, fields: &[&str]) -> bool {
    let hay: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    query.split_whitespace().all(|t| { let t = t.to_lowercase(); hay.iter().any(|h| h.contains(&t)) })
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn terms_are_anded_and_case_insensitive() {
        let f = ["Fix login bug", "gitlab.corp", "grp/proj", "bob"];
        assert!(matches("", &f) && matches("   ", &f));
        assert!(matches("LOGIN", &f) && matches("login bob", &f) && matches("proj corp", &f));
        assert!(!matches("login alice", &f) && !matches("zzz", &f));
    }
}
