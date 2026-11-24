//! Review view model: file list, batches, viewed progress, quit prompt.
use perusal_core::store::Store;
use perusal_diff::{group_files, FileStat};
use std::collections::HashSet;

pub struct ReviewVm {
    pub review: i64,
    pub files: Vec<FileStat>,
    pub filter: String,
    pub sort: String,
    pub grouping: bool,
    /// Order files as the directory tree shows them (folders first), so n/p follow the Files panel.
    pub dir_order: bool,
    pub exclude_noisy_from_progress: bool,
    /// When set, only these paths (a commit or commit range) are listed.
    pub commit_scope: Option<HashSet<String>>,
    pub visible: Vec<usize>,
    batches: Vec<Vec<usize>>,
    pub cursor: usize,
    /// "mark as viewed" toggle carried by the next-batch button.
    pub mark_on_next: bool,
    history: Vec<usize>,
}

#[derive(Debug, PartialEq)]
pub enum QuitChoice { Submit, KeepForLater, Discard }

impl ReviewVm {
    pub fn new(store: &Store, review: i64, mut files: Vec<FileStat>) -> Self {
        let viewed = store.viewed(review).unwrap_or_default();
        for f in &mut files { f.viewed = viewed.contains(&f.path); }
        let mut vm = ReviewVm { review, files, filter: String::new(), sort: String::new(), grouping: false, dir_order: false, exclude_noisy_from_progress: false,
            commit_scope: None, visible: vec![], batches: vec![], cursor: 0, mark_on_next: true, history: vec![] };
        let _ = vm.refresh(); vm
    }
    /// Recomputes visible files and batches after any filter/sort/scope change.
    pub fn refresh(&mut self) -> Result<(), String> {
        let mut idx = perusal_filter::filter(&self.files, &self.filter)?;
        if let Some(scope) = &self.commit_scope { idx.retain(|&i| scope.contains(&self.files[i].path)); }
        perusal_filter::sort(&self.files, &mut idx, &self.sort)?;
        if self.dir_order { idx.sort_by_cached_key(|&i| crate::tree::tree_key(&self.files[i].path)); }
        self.batches = if self.grouping {
            let sel: Vec<FileStat> = idx.iter().map(|&i| self.files[i].clone()).collect();
            group_files(&sel).into_iter().map(|g| g.into_iter().map(|j| idx[j]).collect()).collect()
        } else { idx.iter().map(|&i| vec![i]).collect() };
        self.visible = idx; self.cursor = self.cursor.min(self.batches.len().saturating_sub(1)); Ok(())
    }
    pub fn set_filter(&mut self, f: &str) -> Result<(), String> {
        let old = std::mem::replace(&mut self.filter, f.into());
        self.refresh().inspect_err(|_| { self.filter = old; })
    }
    /// Moves the cursor to the batch containing file `i` (clicking a file in the list).
    pub fn select_file(&mut self, i: usize) {
        if let Some(b) = self.batches.iter().position(|b| b.contains(&i)) { if b != self.cursor { self.history.push(self.cursor); } self.cursor = b; }
    }
    pub fn current_batch(&self) -> &[usize] { self.batches.get(self.cursor).map(|v| v.as_slice()).unwrap_or(&[]) }
    pub fn batch_count(&self) -> usize { self.batches.len() }
    pub fn set_viewed(&mut self, store: &Store, i: usize, v: bool) {
        self.files[i].viewed = v; let _ = store.set_viewed(self.review, &self.files[i].path.clone(), v);
    }
    /// Next batch: applies the viewed toggle to the current batch, then advances. Returns false at the end.
    pub fn next(&mut self, store: &Store) -> bool {
        if self.mark_on_next { for i in self.current_batch().to_vec() { self.set_viewed(store, i, true); } }
        if self.cursor + 1 >= self.batches.len() { return false; }
        self.history.push(self.cursor); self.cursor += 1; true
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
        let vm = ReviewVm::new(&s, r, fs); (s, vm)
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
    #[test] fn dir_order_follows_tree() {
        let (_s, mut vm) = setup(); vm.dir_order = true; vm.refresh().unwrap();
        let order: Vec<&str> = vm.visible.iter().map(|&i| vm.files[i].path.as_str()).collect();
        assert_eq!(order, vec!["a/foo.h", "b/foo.cpp", "c/bar.rs", "d/baz.rs", "Cargo.lock"]);
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
