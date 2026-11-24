//! Agent adapters, ACP client, safety rules and full-review parsing. UI-free.
pub mod acp;
pub mod safety;

use serde::Deserialize;

pub const REVIEW_SKILL: &str = include_str!("../../../skills/review.md");

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentKind { ClaudeCode, Gemini, Codex, OpenCode }

#[derive(Debug, Clone, PartialEq)]
pub struct Launch { pub program: String, pub args: Vec<String>, pub acp: bool }

impl AgentKind {
    /// ACP launch where the agent speaks it; headless CLI otherwise.
    pub fn launch(self, prompt: &str, prefer_acp: bool) -> Launch {
        let p = |s: &str| s.to_string();
        match (self, prefer_acp) {
            (AgentKind::Gemini, true) => Launch { program: p("gemini"), args: vec![p("--experimental-acp")], acp: true },
            (AgentKind::OpenCode, true) => Launch { program: p("opencode"), args: vec![p("acp")], acp: true },
            (AgentKind::ClaudeCode, _) => Launch { program: p("claude"), args: vec![p("-p"), p(prompt), p("--permission-mode"), p("plan")], acp: false },
            (AgentKind::Gemini, _) => Launch { program: p("gemini"), args: vec![p("-p"), p(prompt)], acp: false },
            (AgentKind::Codex, _) => Launch { program: p("codex"), args: vec![p("exec"), p("--sandbox"), p("read-only"), p(prompt)], acp: false },
            (AgentKind::OpenCode, _) => Launch { program: p("opencode"), args: vec![p("run"), p(prompt)], acp: false },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AiComment { pub file: String, pub start_line: u32, pub end_line: u32, pub severity: String, pub body: String }

/// Extracts the JSON array from agent output (possibly fenced or surrounded by prose). Output is untrusted:
/// unknown severities are rejected, bodies are length-capped, and the result is only ever stored as drafts.
pub fn parse_review(output: &str) -> Result<Vec<AiComment>, String> {
    let start = output.find('[').ok_or("no JSON array in agent output")?;
    let end = output.rfind(']').ok_or("no JSON array in agent output")?;
    if end < start { return Err("malformed JSON array".into()); }
    let mut v: Vec<AiComment> = serde_json::from_str(&output[start..=end]).map_err(|e| format!("invalid review JSON: {e}"))?;
    for c in &mut v {
        if !["blocker", "major", "minor", "nit"].contains(&c.severity.as_str()) { return Err(format!("unknown severity '{}'", c.severity)); }
        if c.start_line == 0 || c.end_line < c.start_line { return Err(format!("bad line range in {}", c.file)); }
        if c.body.len() > 4000 { c.body.truncate(4000); }
    }
    Ok(v)
}

/// Draft text for an AI comment (severity prefix keeps the tag visible when exported).
pub fn draft_body(c: &AiComment) -> String { format!("[{}] {}", c.severity, c.body) }

/// Headless CLI fallback: runs the agent once in `cwd` with a minimal environment and returns stdout.
pub fn run_headless(l: &Launch, cwd: &std::path::Path, env: &std::collections::BTreeMap<String, String>) -> std::io::Result<String> {
    let o = std::process::Command::new(&l.program).args(&l.args).current_dir(cwd).env_clear().envs(env).stdin(std::process::Stdio::null()).output()?;
    if !o.status.success() { return Err(std::io::Error::other(String::from_utf8_lossy(&o.stderr).chars().take(300).collect::<String>())); }
    Ok(String::from_utf8_lossy(&o.stdout).to_string())
}

#[cfg(test)]
mod tests {
    #[test] fn headless_fallback_runs_with_minimal_env() {
        let l = super::Launch { program: "sh".into(), args: vec!["-c".into(), "echo \"[]$SECRET\"".into()], acp: false };
        let env = [("PATH".to_string(), std::env::var("PATH").unwrap())].into();
        assert_eq!(super::run_headless(&l, &std::env::temp_dir(), &env).unwrap().trim(), "[]");
    }
    use super::*;
    #[test] fn launches() {
        assert!(AgentKind::Gemini.launch("x", true).acp); assert!(!AgentKind::ClaudeCode.launch("x", true).acp);
        let c = AgentKind::Codex.launch("hello", false); assert!(c.args.contains(&"read-only".to_string()) && c.args.contains(&"hello".to_string()));
    }
    #[test] fn parses_fenced_and_rejects_bad() {
        let ok = "Here:\n```json\n[{\"file\":\"a.rs\",\"start_line\":2,\"end_line\":3,\"severity\":\"major\",\"body\":\"bug\"}]\n```";
        let v = parse_review(ok).unwrap(); assert_eq!(v.len(), 1); assert_eq!(draft_body(&v[0]), "[major] bug");
        assert_eq!(parse_review("[]").unwrap().len(), 0);
        assert!(parse_review("nothing").is_err());
        assert!(parse_review("[{\"file\":\"a\",\"start_line\":1,\"end_line\":1,\"severity\":\"huge\",\"body\":\"x\"}]").is_err());
        assert!(parse_review("[{\"file\":\"a\",\"start_line\":5,\"end_line\":1,\"severity\":\"nit\",\"body\":\"x\"}]").is_err());
    }
    #[test] fn skill_ships() { assert!(REVIEW_SKILL.contains("high-signal") && REVIEW_SKILL.contains("untrusted")); }
}
