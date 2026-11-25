//! Prompt templates: Markdown files with `{{field}}` placeholders. Built-in defaults ship in the binary; a file with the
//! same name in the user's prompts folder replaces it, so experienced users can customise every prompt.
use std::path::{Path, PathBuf};

/// The prompts the app sends to an agent. `file()` is the template's name in the prompts folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind { Chat, Review, GenerateFilter, GenerateSort, GenerateGroupRule }

impl PromptKind {
    pub const ALL: [PromptKind; 5] = [PromptKind::Chat, PromptKind::Review, PromptKind::GenerateFilter, PromptKind::GenerateSort, PromptKind::GenerateGroupRule];
    pub fn file(self) -> &'static str {
        match self { PromptKind::Chat => "chat.md", PromptKind::Review => "review.md", PromptKind::GenerateFilter => "generate-filter.md",
            PromptKind::GenerateSort => "generate-sort.md", PromptKind::GenerateGroupRule => "generate-group-rule.md" }
    }
    pub fn default_template(self) -> &'static str {
        match self {
            PromptKind::Chat => include_str!("../prompts/chat.md"), PromptKind::Review => include_str!("../prompts/review.md"),
            PromptKind::GenerateFilter => include_str!("../prompts/generate-filter.md"), PromptKind::GenerateSort => include_str!("../prompts/generate-sort.md"),
            PromptKind::GenerateGroupRule => include_str!("../prompts/generate-group-rule.md"),
        }
    }
}

/// Documentation for agents and humans editing the templates; written next to them.
pub const PROMPTS_AGENTS_MD: &str = include_str!("../prompts/AGENTS.md");

/// Replaces `{{name}}` with the matching value in one pass; values are never re-scanned (they may hold untrusted text)
/// and unknown placeholders stay as written.
pub fn fill(template: &str, fields: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) => match fields.iter().find(|(k, _)| *k == after[..j].trim()) {
                Some((_, v)) => { out.push_str(v); rest = &after[j + 2..]; }
                None => { out.push_str("{{"); rest = after; }
            },
            None => { out.push_str(&rest[i..]); rest = ""; }
        }
    }
    out + rest
}

/// Where templates are looked up. `Prompts::default()` uses only the built-ins (tests, headless tools).
#[derive(Debug, Clone, Default)]
pub struct Prompts { pub dir: Option<PathBuf> }

impl Prompts {
    pub fn new(dir: impl Into<PathBuf>) -> Self { Prompts { dir: Some(dir.into()) } }
    /// The user's template when the file exists and has content, otherwise the built-in one.
    pub fn template(&self, kind: PromptKind) -> String {
        self.dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(kind.file())).ok()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| kind.default_template().to_string())
    }
    pub fn render(&self, kind: PromptKind, fields: &[(&str, String)]) -> String { fill(&self.template(kind), fields) }
}

/// Creates the folder and writes every default template plus `AGENTS.md` that is not there yet. Never overwrites.
pub fn ensure_prompt_files(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let put = |name: &str, body: &str| { let f = dir.join(name); if !f.exists() { let _ = std::fs::write(f, body); } };
    for k in PromptKind::ALL { put(k.file(), k.default_template()); }
    put("AGENTS.md", PROMPTS_AGENTS_MD);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn fill_is_single_pass_and_keeps_unknown() {
        let f = [("a", "{{b}}".to_string()), ("b", "B".to_string())];
        assert_eq!(fill("x {{a}} {{ b }} {{nope}} {{open", &f), "x {{b}} B {{nope}} {{open");
    }
    #[test] fn user_file_overrides_and_empty_falls_back() {
        let d = std::env::temp_dir().join(format!("perusal-prompts-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d);
        let p = Prompts::new(&d);
        assert_eq!(p.template(PromptKind::Chat), PromptKind::Chat.default_template());
        ensure_prompt_files(&d);
        for k in PromptKind::ALL { assert!(d.join(k.file()).exists()); } assert!(d.join("AGENTS.md").exists());
        std::fs::write(d.join("chat.md"), "custom {{question}}").unwrap();
        ensure_prompt_files(&d); // does not overwrite
        assert_eq!(p.render(PromptKind::Chat, &[("question", "why?".into())]), "custom why?");
        std::fs::write(d.join("chat.md"), "  \n").unwrap();
        assert_eq!(p.template(PromptKind::Chat), PromptKind::Chat.default_template());
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test] fn every_documented_field_is_in_agents_md_and_used() {
        for f in ["selection", "history", "question", "overview", "threads", "drafts", "changed_files", "patch", "request", "reference"] { assert!(PROMPTS_AGENTS_MD.contains(&format!("{{{{{f}}}}}")), "{f}"); }
        for k in PromptKind::ALL { assert!(k.default_template().contains("{{"), "{}", k.file()); }
    }
}
