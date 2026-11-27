//! Git graph view model: click a commit or select a range to scope the review to it.
use perusal_gitops::GraphRow;
use std::collections::HashSet;
use std::path::Path;

pub struct GraphVm { pub rows: Vec<GraphRow>, pub selection: Option<(usize, usize)> }

impl GraphVm {
    pub fn load(dir: &Path, base: &str, head: &str, ctx: usize) -> Result<Self, String> { Ok(GraphVm { rows: perusal_gitops::graph(dir, base, head, ctx)?, selection: None }) }
    /// Click a single commit.
    pub fn click(&mut self, i: usize) { self.selection = Some((i, i)); }
    /// Select a contiguous range of rows (inclusive); only MR commits can be selected.
    pub fn select_range(&mut self, a: usize, b: usize) { let (lo, hi) = (a.min(b), a.max(b)); if self.rows.get(lo..=hi).map(|r| r.iter().all(|x| x.in_mr)).unwrap_or(false) { self.selection = Some((lo, hi)); } }
    /// The selected range as (newest, oldest) commit hashes, for remembering it across sessions.
    pub fn selected_shas(&self) -> Option<(String, String)> { let (lo, hi) = self.selection?; Some((self.rows.get(lo)?.sha.clone(), self.rows.get(hi)?.sha.clone())) }
    /// Re-selects a remembered range by hash; false (and nothing selected) when a commit is gone or not part of the review.
    pub fn restore(&mut self, newest: &str, oldest: &str) -> bool {
        let at = |s: &str| self.rows.iter().position(|r| r.sha == s);
        let (Some(a), Some(b)) = (at(newest), at(oldest)) else { return false };
        let before = self.selection; self.selection = None; self.select_range(a, b);
        if self.selection.is_none() { self.selection = before; false } else { true }
    }
    pub fn clear(&mut self) { self.selection = None; }
    /// The revisions the diffs should compare while a commit or range is selected: (parent of the oldest selected commit, newest selected commit).
    pub fn revs(&self) -> Option<(String, String)> {
        let (lo, hi) = self.selection?;
        if !self.rows.get(lo)?.in_mr { return None; }
        Some((format!("{}^", self.rows.get(hi)?.sha), self.rows[lo].sha.clone()))
    }
    /// Paths to scope the file list to; `None` when nothing is selected.
    pub fn scope(&self, dir: &Path) -> Result<Option<HashSet<String>>, String> {
        let Some((lo, hi)) = self.selection else { return Ok(None) };
        if !self.rows[lo].in_mr { return Ok(None); }
        let (newest, oldest) = (&self.rows[lo].sha, &self.rows[hi].sha);
        let files = if lo == hi { perusal_gitops::files_in(dir, None, newest)? }
            else { perusal_gitops::files_in(dir, Some(&format!("{oldest}^")), newest)? };
        Ok(Some(files.into_iter().collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(sha: &str, in_mr: bool) -> GraphRow { GraphRow { sha: sha.into(), parents: vec![], subject: String::new(), in_mr, lane: 0, author: String::new(), email: String::new() } }
    #[test] fn selection_survives_by_hash() {
        let mut g = GraphVm { rows: vec![row("c", true), row("b", true), row("a", false)], selection: None };
        g.select_range(0, 1); let (n, o) = g.selected_shas().unwrap(); assert_eq!((n.as_str(), o.as_str()), ("c", "b"));
        let mut h = GraphVm { rows: vec![row("d", true), row("c", true), row("b", true), row("a", false)], selection: None };
        assert!(h.restore(&n, &o)); assert_eq!(h.selection, Some((1, 2)));
        assert!(!h.restore("zzz", "b")); assert!(!h.restore("c", "a")); // gone, or context-only: selection untouched
        assert_eq!(h.selection, Some((1, 2)));
    }
}
