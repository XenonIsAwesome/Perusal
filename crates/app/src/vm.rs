//! Review view model: file list, batches, viewed progress, quit prompt.
use perusal_core::store::Store;
use perusal_diff::{group_files, FileStat, GroupRule};
use std::collections::HashSet;

pub struct ReviewVm {
    pub review: i64,
    pub files: Vec<FileStat>,
    pub filter: String,
    pub sort: String,
    pub grouping: bool,
    /// Rules used when `grouping` is on.
    pub rules: Vec<GroupRule>,
    /// Order files as the directory tree shows them (folders first), so n/p follow the Files panel.
    pub dir_order: bool,
    pub exclude_noisy_from_progress: bool,
    /// Hide files whose changes are whitespace only.
    pub hide_whitespace: bool,
    /// When set, only these paths (a commit or commit range) are listed.
    pub commit_scope: Option<HashSet<String>>,
    pub visible: Vec<usize>,
    batches: Vec<Vec<usize>>,
    pub cursor: usize,
    /// "mark as viewed" toggle carried by the next-batch button.
    pub mark_on_next: bool,
    history: Vec<usize>,
    /// Batches the user built by hand (ctrl/shift-click). They override automatic grouping for their members.
    pub manual: Vec<Vec<usize>>,
}

#[derive(Debug, PartialEq)]
pub enum QuitChoice { Submit, KeepForLater, Discard }

impl ReviewVm {
    pub fn new(store: &Store, review: i64, mut files: Vec<FileStat>) -> Self {
        let viewed = store.viewed(review).unwrap_or_default();
        for f in &mut files { f.viewed = viewed.contains(&f.path); }
        let mut vm = ReviewVm { review, files, filter: String::new(), sort: String::new(), grouping: false, rules: perusal_diff::rules::builtin_rules(), dir_order: false, exclude_noisy_from_progress: false, hide_whitespace: false,
            commit_scope: None, visible: vec![], batches: vec![], cursor: 0, mark_on_next: false, history: vec![], manual: vec![] };
        let _ = vm.refresh(); vm
    }
    /// Recomputes visible files and batches after any filter/sort/scope change.
    pub fn refresh(&mut self) -> Result<(), String> {
        let mut idx = perusal_filter::filter(&self.files, &self.filter)?;
        if let Some(scope) = &self.commit_scope { idx.retain(|&i| scope.contains(&self.files[i].path)); }
        if self.hide_whitespace { idx.retain(|&i| !self.files[i].ws_only()); }
        perusal_filter::sort(&self.files, &mut idx, &self.sort)?;
        if self.dir_order { idx.sort_by_cached_key(|&i| crate::tree::tree_key(&self.files[i].path)); }
        self.batches = if self.grouping {
            let sel: Vec<FileStat> = idx.iter().map(|&i| self.files[i].clone()).collect();
            group_files(&sel, &self.rules).into_iter().map(|g| g.into_iter().map(|j| idx[j]).collect()).collect()
        } else { idx.iter().map(|&i| vec![i]).collect() };
        // manual groups: members (that are visible) form one batch at the position of the first member
        for g in &self.manual {
            let members: Vec<usize> = idx.iter().copied().filter(|i| g.contains(i)).collect();
            if members.len() < 2 { continue; }
            let Some(pos) = self.batches.iter().position(|b| b.iter().any(|i| members.contains(i))) else { continue };
            for b in &mut self.batches { b.retain(|i| !members.contains(i)); }
            self.batches.insert(pos, members);
            self.batches.retain(|b| !b.is_empty());
        }
        self.visible = idx; self.cursor = self.cursor.min(self.batches.len().saturating_sub(1)); Ok(())
    }
    /// Sets the sort expression, keeping the old one if the new one is invalid.
    pub fn set_sort(&mut self, f: &str) -> Result<(), String> {
        let old = std::mem::replace(&mut self.sort, f.into());
        self.refresh().inspect_err(|_| { self.sort = old; })
    }
    pub fn set_filter(&mut self, f: &str) -> Result<(), String> {
        let old = std::mem::replace(&mut self.filter, f.into());
        self.refresh().inspect_err(|_| { self.filter = old; })
    }
    /// Refreshes per-file comment counts and the AI flag from saved drafts, so filters like `ai` and `comments > 0` work.
    pub fn sync_review_state(&mut self, store: &Store) {
        let drafts = store.drafts(self.review).unwrap_or_default();
        for f in &mut self.files {
            let mine: Vec<_> = drafts.iter().filter(|d| d.file.as_deref() == Some(f.path.as_str())).collect();
            f.comments = mine.len() as u32; f.ai_flagged = mine.iter().any(|d| d.ai);
        }
    }
    /// Makes `members` one batch (needs at least two). Files already in another manual group leave it.
    pub fn group_files_manually(&mut self, members: Vec<usize>) {
        if members.len() < 2 { return; }
        for g in &mut self.manual { g.retain(|i| !members.contains(i)); }
        self.manual.retain(|g| g.len() >= 2); self.manual.push(members.clone());
        let _ = self.refresh(); self.select_file(members[0]);
    }
    /// Manual groups as file paths (stable across reloads).
    pub fn manual_paths(&self) -> Vec<Vec<String>> { self.manual.iter().map(|g| g.iter().filter_map(|&i| self.files.get(i).map(|f| f.path.clone())).collect()).collect() }
    /// Restores manual groups saved by path; paths that are no longer in the review are skipped.
    pub fn set_manual_paths(&mut self, groups: Vec<Vec<String>>) {
        self.manual = groups.into_iter().map(|g| g.iter().filter_map(|p| self.files.iter().position(|f| &f.path == p)).collect::<Vec<_>>()).filter(|g| g.len() >= 2).collect();
        let _ = self.refresh();
    }
    /// Dissolves the manual group containing `fi`.
    pub fn ungroup(&mut self, fi: usize) { self.manual.retain(|g| !g.contains(&fi)); let _ = self.refresh(); }
    pub fn is_manually_grouped(&self, fi: usize) -> bool { self.manual.iter().any(|g| g.contains(&fi)) }
    /// Moves the cursor to the batch containing file `i` (clicking a file in the list).
    pub fn select_file(&mut self, i: usize) {
        if let Some(b) = self.batches.iter().position(|b| b.contains(&i)) { if b != self.cursor { self.history.push(self.cursor); } self.cursor = b; }
    }
    pub fn current_batch(&self) -> &[usize] { self.batches.get(self.cursor).map(|v| v.as_slice()).unwrap_or(&[]) }
    pub fn batches_contain_together(&self, a: usize, b: usize) -> bool { self.batches.iter().any(|x| x.contains(&a) && x.contains(&b)) }
    pub fn batch_count(&self) -> usize { self.batches.len() }
    pub fn set_viewed(&mut self, store: &Store, i: usize, v: bool) {
        self.files[i].viewed = v; let _ = store.set_viewed(self.review, &self.files[i].path.clone(), v);
    }
    /// Next batch: applies the viewed toggle to the current batch, then advances; after the last batch it wraps to the first.
    /// Returns false only when there is nowhere else to go.
    pub fn next(&mut self, store: &Store) -> bool {
        if self.mark_on_next { for i in self.current_batch().to_vec() { self.set_viewed(store, i, true); } }
        if self.batches.len() < 2 { return false; }
        self.history.push(self.cursor); self.cursor = (self.cursor + 1) % self.batches.len(); true
    }
    /// Back to the previous batch (covers a forgotten comment).
    pub fn back(&mut self) -> bool { if let Some(c) = self.history.pop() { self.cursor = c; true } else { false } }
    /// (viewed, total) honoring the noisy-file setting.
    pub fn progress(&self) -> (usize, usize) {
        let it = self.files.iter().filter(|f| !(self.exclude_noisy_from_progress && f.noisy()));
        let (mut v, mut t) = (0, 0); for f in it { t += 1; if f.viewed { v += 1; } } (v, t)
    }
    /// Quit prompt is needed only when there are unsubmitted drafts.
    pub fn needs_quit_prompt(&self, store: &Store) -> bool { store.drafts(self.review).map(|d| !d.is_empty()).unwrap_or(false) }
    pub fn quit(&self, store: &Store, c: QuitChoice) -> Result<(), String> {
        match c { QuitChoice::Discard => store.discard(self.review).map_err(|e| e.to_string()), _ => Ok(()) } // Submit is handled by the caller before; Keep leaves the journal as is.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use perusal_diff::Status;
    fn setup() -> (Store, ReviewVm) {
        let s = Store::memory().unwrap(); let r = s.open_review("k", "t", "h", 5).unwrap();
        let fs = vec![FileStat::new("a/foo.h", Status::Modified, 5, 1), FileStat::new("b/foo.cpp", Status::Modified, 50, 1), FileStat::new("c/bar.rs", Status::Added, 9, 0),
            FileStat::new("Cargo.lock", Status::Modified, 100, 100), FileStat::new("d/baz.rs", Status::Modified, 1, 1)];
        let mut vm = ReviewVm::new(&s, r, fs); vm.mark_on_next = true; (s, vm)
    }
    #[test] fn batch_flow_marks_and_goes_back() {
        let (s, mut vm) = setup();
        assert_eq!(vm.batch_count(), 5); assert_eq!(vm.progress(), (0, 5));
        assert!(vm.next(&s)); assert_eq!(vm.progress(), (1, 5));
        vm.mark_on_next = false; assert!(vm.next(&s)); assert_eq!(vm.progress(), (1, 5));
        assert!(vm.back()); assert_eq!(vm.cursor, 1); assert!(vm.back()); assert!(!vm.back());
        assert_eq!(s.progress(vm.review).unwrap().0, 1); // persisted
    }
    #[test] fn grouping_batches_related_files() {
        let (s, mut vm) = setup(); vm.grouping = true; vm.refresh().unwrap();
        assert_eq!(vm.batch_count(), 4); assert_eq!(vm.current_batch().len(), 2);
        vm.next(&s); assert_eq!(vm.progress().0, 2);
    }
    #[test] fn filter_drives_batches_and_bad_filter_is_rejected() {
        let (_s, mut vm) = setup();
        vm.set_filter("additions > 8").unwrap(); assert_eq!(vm.batch_count(), 3);
        assert!(vm.set_filter("nope >").is_err()); assert_eq!(vm.filter, "additions > 8");
        vm.sort = "additions desc".into(); vm.refresh().unwrap(); assert_eq!(vm.files[vm.current_batch()[0]].path, "Cargo.lock");
    }
    #[test] fn manual_groups_by_path_roundtrip() {
        let (_s, mut vm) = setup(); vm.group_files_manually(vec![2, 4]); let saved = vm.manual_paths(); assert_eq!(saved.len(), 1);
        vm.ungroup(2); assert!(vm.manual.is_empty()); vm.set_manual_paths(saved); assert!(vm.is_manually_grouped(4));
        vm.set_manual_paths(vec![vec!["gone.rs".into(), "also-gone.rs".into()]]); assert!(vm.manual.is_empty());
    }
    #[test] fn manual_groups_override_and_ungroup() {
        let (s, mut vm) = setup(); vm.group_files_manually(vec![2, 4]);
        assert_eq!(vm.batch_count(), 4); assert_eq!(vm.current_batch(), &[2, 4]); assert!(vm.is_manually_grouped(4));
        vm.next(&s); assert_eq!(vm.progress().0, 2); // the whole batch is marked together
        vm.ungroup(2); assert_eq!(vm.batch_count(), 5); assert!(!vm.is_manually_grouped(4));
        vm.group_files_manually(vec![0]); assert_eq!(vm.batch_count(), 5); // a single file is not a group
        vm.grouping = true; vm.refresh().unwrap(); vm.group_files_manually(vec![0, 3]); assert!(vm.batches_contain_together(0, 3));
    }
    #[test] fn ai_flag_and_comment_counts_drive_filters() {
        let (s, mut vm) = setup();
        s.add_draft(vm.review, Some("c/bar.rs"), Some((1, 1)), "x", true).unwrap(); s.add_draft(vm.review, Some("a/foo.h"), None, "y", false).unwrap();
        vm.sync_review_state(&s);
        vm.set_filter("ai").unwrap(); assert_eq!(vm.visible.iter().map(|&i| vm.files[i].path.as_str()).collect::<Vec<_>>(), vec!["c/bar.rs"]);
        vm.set_filter("comments > 0").unwrap(); assert_eq!(vm.visible.len(), 2);
    }
    #[test] fn sort_expression_and_next_does_not_mark_viewed() {
        let (s, mut vm) = setup(); vm.mark_on_next = false;
        vm.set_sort("additions desc").unwrap(); assert_eq!(vm.files[vm.visible[0]].path, "Cargo.lock");
        assert!(vm.set_sort("nope >").is_err()); assert_eq!(vm.sort, "additions desc");
        assert!(vm.next(&s)); assert_eq!(vm.progress().0, 0); // moving on is not the same as having viewed it
    }
    #[test] fn dir_order_follows_tree() {
        let (_s, mut vm) = setup(); vm.dir_order = true; vm.refresh().unwrap();
        let order: Vec<&str> = vm.visible.iter().map(|&i| vm.files[i].path.as_str()).collect();
        assert_eq!(order, vec!["a/foo.h", "b/foo.cpp", "c/bar.rs", "d/baz.rs", "Cargo.lock"]);
    }
    #[test] fn next_wraps_and_whitespace_hides() {
        let (s, mut vm) = setup(); for _ in 0..4 { assert!(vm.next(&s)); } assert_eq!(vm.cursor, 4); assert!(vm.next(&s)); assert_eq!(vm.cursor, 0);
        vm.commit_scope = Some(["c/bar.rs".to_string()].into()); vm.refresh().unwrap(); assert!(!vm.next(&s)); vm.commit_scope = None;
        vm.files[0].meaningful = 0; vm.refresh().unwrap(); assert_eq!(vm.batch_count(), 5); vm.hide_whitespace = true; vm.refresh().unwrap(); assert_eq!(vm.batch_count(), 4);
    }
    #[test] fn noisy_exclusion_and_commit_scope() {
        let (_s, mut vm) = setup(); vm.exclude_noisy_from_progress = true; assert_eq!(vm.progress().1, 4);
        vm.commit_scope = Some(["c/bar.rs".to_string()].into()); vm.refresh().unwrap(); assert_eq!(vm.batch_count(), 1);
    }
    #[test] fn quit_prompt_and_discard() {
        let (s, vm) = setup(); assert!(!vm.needs_quit_prompt(&s));
        s.add_draft(vm.review, None, None, "x", false).unwrap(); assert!(vm.needs_quit_prompt(&s));
        vm.quit(&s, QuitChoice::KeepForLater).unwrap(); assert_eq!(s.recent().unwrap().len(), 1);
        vm.quit(&s, QuitChoice::Discard).unwrap(); assert!(s.recent().unwrap().is_empty());
    }
    #[test] fn resumes_viewed_after_crash() {
        let (s, mut vm) = setup(); vm.next(&s);
        let again = ReviewVm::new(&s, vm.review, vm.files.iter().map(|f| FileStat::new(&f.path, f.status, f.additions, f.deletions)).collect());
        assert_eq!(again.progress().0, 1);
    }
}
