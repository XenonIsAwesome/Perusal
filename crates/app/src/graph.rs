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
