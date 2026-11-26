use perusal_app::{banner::*, graph::GraphVm, vm::ReviewVm};
use perusal_core::store::Store;
use perusal_gitops::{git, show};
use std::path::{Path, PathBuf};

fn g(d: &Path, a: &[&str]) -> String { git(d, ["-c", "user.name=t", "-c", "user.email=t@t"].iter().chain(a).copied().collect::<Vec<_>>().as_slice()).unwrap() }
fn repo(n: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("perusal-fl-{}-{n}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); g(&d, &["init", "-q", "-b", "main"]); d }
fn commit(d: &Path, f: &str, c: &str, m: &str) { std::fs::write(d.join(f), c).unwrap(); g(d, &["add", "-A"]); g(d, &["commit", "-qm", m]); }

#[test] fn commit_by_commit_review_scopes_files() {
    let d = repo("graph"); commit(&d, "a", "1\n", "base"); g(&d, &["checkout", "-qb", "feat"]);
    commit(&d, "x", "x\n", "c1"); commit(&d, "y", "y\n", "c2"); commit(&d, "z", "z\n", "c3");
    let base = perusal_gitops::local_base(&d, "feat", None).unwrap();
    let files = perusal_diff::index_range(&d, &base, "feat").unwrap();
    let s = Store::memory().unwrap(); let r = s.open_review("k", "t", "feat", files.len() as u32).unwrap();
    let mut vm = ReviewVm::new(&s, r, files);
    let mut gv = GraphVm::load(&d, &base, "feat", 3).unwrap();
    gv.click(1); vm.commit_scope = gv.scope(&d).unwrap(); vm.refresh().unwrap(); assert_eq!(vm.visible.len(), 1); assert_eq!(vm.files[vm.visible[0]].path, "y");
    gv.select_range(0, 2); vm.commit_scope = gv.scope(&d).unwrap(); vm.refresh().unwrap(); assert_eq!(vm.visible.len(), 3);
    gv.select_range(2, 3); assert_eq!(gv.selection, Some((0, 2))); // context commit rows cannot be selected
    gv.clear(); vm.commit_scope = gv.scope(&d).unwrap(); vm.refresh().unwrap(); assert_eq!(vm.visible.len(), 3);
    let _ = std::fs::remove_dir_all(&d);
}

#[test] fn reanchor_after_force_push_and_banner() {
    let d = repo("rewrite"); commit(&d, "f.rs", "a\nb\nc\nd\n", "base"); g(&d, &["checkout", "-qb", "feat"]);
    commit(&d, "f.rs", "a\nb\nc\nd\ne\n", "feature work");
    let old_head = g(&d, &["rev-parse", "feat"]);
    // history rewritten: a line inserted at top and "c" edited
    g(&d, &["reset", "-q", "--hard", "main"]); commit(&d, "f.rs", "TOP\na\nb\nC!\nd\ne\n", "rewritten");
    let new_head = g(&d, &["rev-parse", "feat"]);
    let s = Store::memory().unwrap(); let r = s.open_review("k", "t", &old_head, 1).unwrap();
    let keep = s.add_draft(r, Some("f.rs"), Some((2, 2)), "on b", false).unwrap();
    let edited = s.add_draft(r, Some("f.rs"), Some((3, 3)), "on c", false).unwrap();
    let lost = s.move_head(r, &new_head, &|f, old| show(&d, if old { &old_head } else { &new_head }, f)).unwrap();
    assert_eq!(lost, vec![edited]);
    assert_eq!(s.drafts(r).unwrap().iter().find(|x| x.id == keep).unwrap().start_line, Some(3));
    let anc = |a: &str, b: &str| git(&d, &["merge-base", "--is-ancestor", a, b]).is_ok();
    assert_eq!(banner("open", &old_head, &new_head, &anc), Banner::ForcePushed { new_head: new_head.clone() });
    let _ = std::fs::remove_dir_all(&d);
}

#[test] fn move_review_keeps_drafts_and_unmarks_changed_files() {
    use perusal_app::movehead::*;
    let d = repo("move"); commit(&d, "f.rs", "a\nb\nc\nd\n", "base"); commit(&d, "g.rs", "1\n2\n", "g"); g(&d, &["checkout", "-qb", "feat"]);
    commit(&d, "f.rs", "a\nb\nc\nd\ne\n", "feature work"); let old_head = g(&d, &["rev-parse", "feat"]);
    // a plain new commit changes g.rs only
    commit(&d, "g.rs", "1\n2\n3\n", "more"); let new_head = g(&d, &["rev-parse", "feat"]);
    let s = Store::memory().unwrap(); let r = s.open_review("k", "t", &old_head, 2).unwrap();
    s.add_draft(r, Some("f.rs"), Some((2, 2)), "on b", false).unwrap(); s.set_viewed(r, "f.rs", true).unwrap(); s.set_viewed(r, "g.rs", true).unwrap();
    let rep = move_review(&s, r, &d, &old_head, &new_head).unwrap();
    assert!(!rep.force_pushed); assert_eq!(rep.kept, 1); assert!(rep.lost.is_empty()); assert_eq!(rep.unviewed, vec!["g.rs".to_string()]);
    assert_eq!(s.viewed(r).unwrap(), vec!["f.rs".to_string()]); assert_eq!(s.pinned_head(r).unwrap(), new_head);
    // rewritten history is reported as a force-push
    g(&d, &["reset", "-q", "--hard", &old_head]); commit(&d, "f.rs", "TOP\na\nb\nc\nd\ne\n", "rewritten"); let forced = g(&d, &["rev-parse", "feat"]);
    let rep = move_review(&s, r, &d, &new_head, &forced).unwrap(); assert!(rep.force_pushed); assert_eq!(rep.kept, 1);
    let _ = std::fs::remove_dir_all(&d);
}
