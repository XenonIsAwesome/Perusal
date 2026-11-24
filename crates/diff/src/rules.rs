//! File grouping rules: name-based, editable, with user patterns. Each rule is a regex with a `stem` capture;
//! files that match the same enabled rule with the same stem (across folders) are batched together.
use crate::FileStat;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupRule { pub id: String, pub label: String, pub pattern: String, pub enabled: bool, #[serde(default)] pub builtin: bool }

/// Stems too generic to relate files across folders (every crate has a lib.rs, every package an index.ts).
const GENERIC_STEMS: [&str; 12] = ["index", "main", "mod", "lib", "__init__", "utils", "util", "types", "test", "tests", "setup", "config"];

pub fn builtin_rules() -> Vec<GroupRule> {
    let r = |id: &str, label: &str, pattern: &str| GroupRule { id: id.into(), label: label.into(), pattern: pattern.into(), enabled: true, builtin: true };
    vec![
        r("c-cpp", "C and C++ headers with sources (foo.h, foo.cpp)", r"(?i)^(?:.*/)?(?P<stem>[^/]+)\.(?:h|hpp|c|cc|cpp)$"),
        r("ts-tests", "TypeScript and JavaScript with tests (foo.ts, foo.test.ts, foo.spec.ts)", r"^(?:.*/)?(?P<stem>[^/]+?)(?:\.test|\.spec)?\.(?:ts|tsx|js|jsx)$"),
        r("proto", "Protobuf with generated files (foo.proto, foo.pb.go, foo_pb2.py)", r"^(?:.*/)?(?P<stem>[^/]+?)(?:\.proto|\.pb\.(?:go|h|cc)|_pb2(?:_grpc)?\.pyi?|_pb\.js)$"),
        r("py-tests", "Python modules with tests (foo.py, test_foo.py)", r"^(?:.*/)?(?:test_)?(?P<stem>[^/]+?)\.py$"),
        r("rust-tests", "Rust modules with tests (foo.rs, foo_test.rs)", r"^(?:.*/)?(?P<stem>[^/]+?)(?:_test)?\.rs$"),
    ]
}

/// Keeps the user's flags and custom rules, and makes sure every built-in rule is present (new built-ins are added enabled).
pub fn merge_with_builtins(saved: Vec<GroupRule>) -> Vec<GroupRule> {
    let mut out: Vec<GroupRule> = builtin_rules().into_iter().map(|b| match saved.iter().find(|s| s.id == b.id) { Some(s) => GroupRule { enabled: s.enabled, ..b }, None => b }).collect();
    let ids: Vec<String> = out.iter().map(|o| o.id.clone()).collect();
    out.extend(saved.into_iter().filter(|s| !s.builtin && !ids.contains(&s.id)));
    out
}

/// Validates a user pattern: must compile and contain a `stem` group. Returns an actionable message.
pub fn validate_pattern(p: &str) -> Result<(), String> {
    let re = Regex::new(p).map_err(|e| format!("not a valid regex: {}", e.to_string().lines().last().unwrap_or("")))?;
    if re.capture_names().flatten().any(|n| n == "stem") { Ok(()) } else { Err("the pattern needs a named group (?P<stem>...) that captures the shared part of the name".into()) }
}

/// Groups file indexes; files that no enabled rule relates stay alone. Order follows first appearance.
pub fn group_files(files: &[FileStat], rules: &[GroupRule]) -> Vec<Vec<usize>> {
    let compiled: Vec<(&GroupRule, Regex)> = rules.iter().filter(|r| r.enabled).filter_map(|r| Regex::new(&r.pattern).ok().map(|re| (r, re))).collect();
    let mut order: Vec<Vec<usize>> = Vec::new();
    let mut by_key: HashMap<(String, String), usize> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        let key = compiled.iter().find_map(|(r, re)| {
            let stem = re.captures(&f.path)?.name("stem")?.as_str().to_lowercase();
            (!GENERIC_STEMS.contains(&stem.as_str())).then(|| (r.id.clone(), stem))
        });
        match key {
            Some(k) => { let g = *by_key.entry(k).or_insert_with(|| { order.push(vec![]); order.len() - 1 }); order[g].push(i); }
            None => order.push(vec![i]),
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Status;
    fn fs(p: &[&str]) -> Vec<FileStat> { p.iter().map(|p| FileStat::new(p, Status::Modified, 1, 1)).collect() }
    #[test] fn builtin_rules_group_by_stem_across_folders() {
        let f = fs(&["a/foo.h", "b/foo.cpp", "x/foo.test.ts", "y/foo.ts", "t/test_bar.py", "bar.py", "z.rs", "m/z_test.rs", "p/api.proto", "gen/api.pb.go", "gen/api_pb2.py"]);
        let g = group_files(&f, &builtin_rules());
        assert_eq!(g, vec![vec![0, 1], vec![2, 3], vec![4, 5], vec![6, 7], vec![8, 9, 10]]);
    }
    #[test] fn rules_do_not_cross_languages_or_generic_stems() {
        let f = fs(&["a/foo.rs", "b/foo.py", "c/main.rs", "d/main.rs", "e/lib.rs"]);
        assert_eq!(group_files(&f, &builtin_rules()).len(), 5);
    }
    #[test] fn disabling_and_custom_rules() {
        let f = fs(&["a/foo.h", "b/foo.cpp", "docs/x.md", "docs/x.png"]);
        let mut rules = builtin_rules(); rules[0].enabled = false; assert_eq!(group_files(&f, &rules).len(), 4);
        rules.push(GroupRule { id: "docs".into(), label: "docs".into(), pattern: r"^docs/(?P<stem>[^/.]+)\.(?:md|png)$".into(), enabled: true, builtin: false });
        assert_eq!(group_files(&f, &rules), vec![vec![0], vec![1], vec![2, 3]]);
    }
    #[test] fn pattern_validation_and_merge() {
        assert!(validate_pattern(r"^(?P<stem>.+)\.x$").is_ok()); assert!(validate_pattern("(").unwrap_err().contains("valid regex")); assert!(validate_pattern("^a$").unwrap_err().contains("stem"));
        let mut saved = builtin_rules(); saved[1].enabled = false; saved.push(GroupRule { id: "mine".into(), label: "m".into(), pattern: "(?P<stem>x)".into(), enabled: true, builtin: false });
        saved.remove(0); // a built-in missing from an old settings file comes back
        let m = merge_with_builtins(saved); assert_eq!(m.len(), 6); assert!(m[0].id == "c-cpp" && m[0].enabled); assert!(!m[1].enabled); assert_eq!(m[5].id, "mine");
    }
}
