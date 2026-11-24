//! Directory tree for the Files panel: folders first, single-child chains compressed (a/b/c), per-folder aggregates.
use perusal_diff::FileStat;
use std::collections::{BTreeMap, HashSet};

/// Sort key putting folders before files at every level, names case-insensitive.
pub fn tree_key(path: &str) -> Vec<(u8, String, String)> {
    let parts: Vec<&str> = path.split('/').collect();
    let last = parts.len() - 1;
    parts.iter().enumerate().map(|(i, p)| (u8::from(i == last), p.to_lowercase(), p.to_string())).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Dir { path: String, name: String, depth: usize, collapsed: bool, total: usize, viewed: usize, additions: u32, deletions: u32 },
    File { fi: usize, depth: usize },
}

#[derive(Default)]
struct Node { dirs: BTreeMap<(String, String), Node>, files: Vec<usize> }

impl Node {
    fn insert(&mut self, parts: &[&str], fi: usize) {
        if parts.len() == 1 { self.files.push(fi); return; }
        self.dirs.entry((parts[0].to_lowercase(), parts[0].to_string())).or_default().insert(&parts[1..], fi);
    }
    fn collect(&self, out: &mut Vec<usize>) { out.extend(&self.files); for d in self.dirs.values() { d.collect(out); } }
}

/// Builds visible rows for `order` (indexes into `files`, already filtered). `collapsed` holds full folder paths.
pub fn build_rows(files: &[FileStat], order: &[usize], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = Node::default();
    for &fi in order { let parts: Vec<&str> = files[fi].path.split('/').collect(); root.insert(&parts, fi); }
    let mut rows = Vec::new();
    emit(&root, "", 0, files, collapsed, &mut rows);
    rows
}

fn emit(n: &Node, prefix: &str, depth: usize, files: &[FileStat], collapsed: &HashSet<String>, rows: &mut Vec<TreeRow>) {
    for ((_, name), child) in &n.dirs {
        // compress chains of folders that only contain one folder
        let (mut label, mut node) = (name.clone(), child);
        while node.files.is_empty() && node.dirs.len() == 1 { let ((_, nn), c) = node.dirs.iter().next().unwrap(); label = format!("{label}/{nn}"); node = c; }
        let path = if prefix.is_empty() { label.clone() } else { format!("{prefix}/{label}") };
        let mut all = Vec::new(); node.collect(&mut all);
        let is_collapsed = collapsed.contains(&path);
        rows.push(TreeRow::Dir { path: path.clone(), name: label, depth, collapsed: is_collapsed, total: all.len(),
            viewed: all.iter().filter(|&&i| files[i].viewed).count(), additions: all.iter().map(|&i| files[i].additions).sum(), deletions: all.iter().map(|&i| files[i].deletions).sum() });
        if !is_collapsed { emit(node, &path, depth + 1, files, collapsed, rows); }
    }
    let mut fs = n.files.clone();
    fs.sort_by_key(|&i| { let p = &files[i].path; let name = p.rsplit('/').next().unwrap_or(p); (name.to_lowercase(), name.to_string()) });
    for fi in fs { rows.push(TreeRow::File { fi, depth }); }
}

/// Every folder path (as shown, compressed), for "collapse all".
pub fn all_dirs(files: &[FileStat], order: &[usize]) -> Vec<String> {
    build_rows(files, order, &HashSet::new()).into_iter().filter_map(|r| if let TreeRow::Dir { path, .. } = r { Some(path) } else { None }).collect()
}

/// Un-collapses every folder that contains `path`, so the selected file is visible.
pub fn expand_to(collapsed: &mut HashSet<String>, path: &str) { collapsed.retain(|c| !path.starts_with(&format!("{c}/"))); }

#[cfg(test)]
mod tests {
    use super::*;
    use perusal_diff::Status;
    fn fs(paths: &[&str]) -> Vec<FileStat> { paths.iter().map(|p| FileStat::new(p, Status::Modified, 2, 1)).collect() }
    fn names(rows: &[TreeRow], f: &[FileStat]) -> Vec<String> {
        rows.iter().map(|r| match r { TreeRow::Dir { name, depth, collapsed, .. } => format!("{}{}/{}", " ".repeat(*depth), name, if *collapsed { "+" } else { "" }), TreeRow::File { fi, depth } => format!("{}{}", " ".repeat(*depth), f[*fi].name()) }).collect()
    }
    #[test] fn folders_first_compressed_sorted() {
        let f = fs(&["README.md", "src/a/b/c.rs", "src/lib.rs", "src/util/x.rs", "src/util/y.rs", "Cargo.toml", "docs/guide.md"]);
        let order: Vec<usize> = (0..f.len()).collect();
        let rows = build_rows(&f, &order, &HashSet::new());
        assert_eq!(names(&rows, &f), vec!["docs/", " guide.md", "src/", " a/b/", "  c.rs", " util/", "  x.rs", "  y.rs", " lib.rs", "Cargo.toml", "README.md"]);
    }
    #[test] fn collapse_aggregates_and_expand() {
        let mut f = fs(&["src/a.rs", "src/b.rs", "top.rs"]); f[0].viewed = true;
        let order = [0, 1, 2]; let mut c = HashSet::new(); c.insert("src".to_string());
        let rows = build_rows(&f, &order, &c);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], TreeRow::Dir { path: "src".into(), name: "src".into(), depth: 0, collapsed: true, total: 2, viewed: 1, additions: 4, deletions: 2 });
        expand_to(&mut c, "src/b.rs"); assert!(c.is_empty());
        let mut c2: HashSet<String> = ["srcx".to_string()].into(); expand_to(&mut c2, "src/b.rs"); assert_eq!(c2.len(), 1); // prefix without slash must not match
    }
    #[test] fn filter_order_respected_and_all_dirs() {
        let f = fs(&["a/x.rs", "a/y.rs", "b/z.rs"]);
        let rows = build_rows(&f, &[2], &HashSet::new()); assert_eq!(names(&rows, &f), vec!["b/", " z.rs"]);
        assert_eq!(all_dirs(&f, &[0, 1, 2]), vec!["a", "b"]);
    }
    #[test] fn key_orders_dirs_before_files() {
        let mut v = vec!["z.rs", "src/b.rs", "a/deep/x.rs", "src/a.rs", "B.md"]; v.sort_by_key(|p| tree_key(p));
        assert_eq!(v, vec!["a/deep/x.rs", "src/a.rs", "src/b.rs", "B.md", "z.rs"]);
    }
}
