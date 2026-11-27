//! Moving a pinned review to the MR's new head: drafts are re-anchored where their lines survive, viewed marks are
//! kept only for files whose content did not change, and a force-push is told apart from plain new commits.
use perusal_core::store::Store;
use std::path::Path;

#[derive(Debug, Default, PartialEq)]
pub struct MoveReport {
    /// The old head is not an ancestor of the new one (history was rewritten).
    pub force_pushed: bool,
    /// Drafts that stayed anchored (possibly on shifted lines).
    pub kept: usize,
    /// Drafts whose lines changed or vanished; they stay in the review and are listed for manual placement.
    pub lost: Vec<i64>,
    /// Viewed marks dropped because the file changed.
    pub unviewed: Vec<String>,
}

/// `git merge-base --is-ancestor old new`
pub fn is_ancestor(repo: &Path, old: &str, new: &str) -> bool { perusal_gitops::git(repo, &["merge-base", "--is-ancestor", old, new]).is_ok() }

/// Re-pins `review` from `old_head` to `new_head`. Both heads must be present in `repo`.
pub fn move_review(store: &Store, review: i64, repo: &Path, old_head: &str, new_head: &str) -> Result<MoveReport, String> {
    if old_head == new_head { return Ok(MoveReport::default()); }
    let content = |f: &str, old: bool| perusal_gitops::show(repo, if old { old_head } else { new_head }, f);
    let before = store.drafts(review).map_err(|e| e.to_string())?.iter().filter(|d| d.file.is_some() && d.start_line.is_some()).count();
    let lost = store.move_head(review, new_head, &content).map_err(|e| e.to_string())?;
    let mut unviewed = vec![];
    for p in store.viewed(review).map_err(|e| e.to_string())? {
        if content(&p, true) != content(&p, false) { store.set_viewed(review, &p, false).map_err(|e| e.to_string())?; unviewed.push(p); }
    }
    Ok(MoveReport { force_pushed: !is_ancestor(repo, old_head, new_head), kept: before.saturating_sub(lost.len()), lost, unviewed })
}

/// One line for the toast after moving.
pub fn summary(r: &MoveReport) -> String {
    let mut s = String::from(if r.force_pushed { "Moved to the force-pushed head." } else { "Moved to the latest commit." });
    if r.kept > 0 { s += &format!(" {} comment{} kept.", r.kept, if r.kept == 1 { "" } else { "s" }); }
    if !r.lost.is_empty() { s += &format!(" {} draft{} need{} a new place: check {}.", r.lost.len(), if r.lost.len() == 1 { "" } else { "s" }, if r.lost.len() == 1 { "s" } else { "" }, if r.lost.len() == 1 { "it" } else { "them" }); }
    if !r.unviewed.is_empty() { s += &format!(" {} changed file{} marked unviewed.", r.unviewed.len(), if r.unviewed.len() == 1 { "" } else { "s" }); }
    s
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn summary_text() {
        assert_eq!(summary(&MoveReport::default()), "Moved to the latest commit.");
        let r = MoveReport { force_pushed: true, kept: 2, lost: vec![5], unviewed: vec!["a".into()] };
        assert_eq!(summary(&r), "Moved to the force-pushed head. 2 comments kept. 1 draft needs a new place: check it. 1 changed file marked unviewed.");
    }
}
