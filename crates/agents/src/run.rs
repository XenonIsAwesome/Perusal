//! Running an agent process with live activity events and cancellation.
use crate::{safety, Launch};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// What the agent is doing right now ("Reading src/lib.rs", "Searching for 'hit_rate'").
    Activity(String),
    /// A piece of the answer.
    Text(String),
    /// The complete answer (when the agent reports one).
    Final(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunError { Cancelled, TimedOut(u64), Failed(String) }
impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self { RunError::Cancelled => write!(f, "Stopped."), RunError::TimedOut(s) => write!(f, "The agent did not finish within {s}s and was stopped."), RunError::Failed(m) => write!(f, "{m}") }
    }
}

fn base(p: &str) -> &str { p.rsplit('/').next().unwrap_or(p) }

fn describe_tool(name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match name {
        "Read" => format!("Reading {}", base(s("file_path"))),
        "Grep" => format!("Searching for '{}'", s("pattern")),
        "Glob" => format!("Finding files matching {}", s("pattern")),
        "LS" => format!("Listing {}", base(s("path"))),
        "Bash" => format!("Running {}", s("command").chars().take(60).collect::<String>()),
        "WebSearch" => format!("Searching the web for '{}'", s("query")),
        "WebFetch" => format!("Fetching {}", s("url")),
        "Task" => "Delegating to a sub-agent".to_string(),
        other => format!("Using {other}"),
    }
}

/// Parses one line of Claude Code `--output-format stream-json`.
pub fn parse_claude_line(line: &str) -> Vec<AgentEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return vec![] };
    let mut out = vec![];
    match v["type"].as_str() {
        Some("assistant") => {
            for c in v.pointer("/message/content").and_then(|c| c.as_array()).into_iter().flatten() {
                match c["type"].as_str() {
                    Some("text") => { if let Some(t) = c["text"].as_str() { out.push(AgentEvent::Text(t.to_string())); } }
                    Some("tool_use") => out.push(AgentEvent::Activity(describe_tool(c["name"].as_str().unwrap_or("tool"), &c["input"]))),
                    Some("thinking") => out.push(AgentEvent::Activity("Thinking".into())),
                    _ => {}
                }
            }
        }
        Some("result") => { if let Some(r) = v["result"].as_str() { out.push(AgentEvent::Final(r.to_string())); } }
        _ => {}
    }
    out
}

/// Runs the agent in `cwd` with a minimal environment (plus the launch's own extras). Calls `on_event` as output
/// arrives. Returns the final answer. Setting `cancel` kills the process.
pub fn run_streaming(l: &Launch, cwd: &Path, env: &BTreeMap<String, String>, timeout: Duration, cancel: &AtomicBool, mut on_event: impl FnMut(AgentEvent)) -> Result<String, RunError> {
    let mut child = Command::new(crate::find_program(&l.program).unwrap_or_else(|| l.program.clone().into())).args(&l.args).current_dir(cwd).env_clear().envs(env).envs(l.env.iter().cloned())
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| RunError::Failed(format!("cannot start {}: {e}", l.program)))?;
    let (tx, rx) = mpsc::channel::<String>();
    let out = child.stdout.take().unwrap();
    std::thread::spawn(move || { for line in BufReader::new(out).lines().map_while(Result::ok) { if tx.send(line).is_err() { break; } } });
    let mut err = child.stderr.take().unwrap();
    let errbuf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    { let eb = errbuf.clone(); std::thread::spawn(move || { let mut s = String::new(); let _ = err.read_to_string(&mut s); *eb.lock().unwrap() = s; }); }
    let start = Instant::now();
    let (mut text, mut fin) = (String::new(), None::<String>);
    loop {
        if cancel.load(Ordering::Relaxed) { let _ = child.kill(); let _ = child.wait(); return Err(RunError::Cancelled); }
        if start.elapsed() > timeout { let _ = child.kill(); let _ = child.wait(); return Err(RunError::TimedOut(timeout.as_secs())); }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                let events = if l.stream_json { parse_claude_line(&line) } else { vec![AgentEvent::Text(line)] };
                for ev in events {
                    match &ev { AgentEvent::Text(t) => { text.push_str(t); text.push('\n'); } AgentEvent::Final(f) => fin = Some(f.clone()), _ => {} }
                    on_event(ev);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().map_err(|e| RunError::Failed(e.to_string()))?;
    std::thread::sleep(Duration::from_millis(20));
    let answer = fin.unwrap_or(text);
    if !status.success() && answer.trim().is_empty() {
        let e = errbuf.lock().unwrap().clone();
        return Err(RunError::Failed(if e.trim().is_empty() { format!("{} exited with {status}", l.program) } else { e.chars().take(500).collect() }));
    }
    Ok(answer)
}

/// Environment passed to agents: minimal, and without anything that looks like a credential.
pub fn agent_env() -> BTreeMap<String, String> {
    let mut e = safety::minimal_env(&std::env::vars().collect());
    // the same directories on_path searches, so shims and launchers find their interpreters
    if let Ok(j) = std::env::join_paths(crate::search_dirs()) { e.insert("PATH".into(), j.to_string_lossy().into_owned()); }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn sh(script: &str, stream: bool) -> Launch { Launch { program: "sh".into(), args: vec!["-c".into(), script.into()], acp: false, env: vec![], stream_json: stream } }
    fn env() -> BTreeMap<String, String> { [("PATH".to_string(), std::env::var("PATH").unwrap())].into() }
    #[test] fn claude_stream_events() {
        let ev = parse_claude_line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/w/src/lib.rs"}},{"type":"text","text":"hi"}]}}"#);
        assert_eq!(ev, vec![AgentEvent::Activity("Reading lib.rs".into()), AgentEvent::Text("hi".into())]);
        assert_eq!(parse_claude_line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Grep","input":{"pattern":"hit_rate"}}]}}"#), vec![AgentEvent::Activity("Searching for 'hit_rate'".into())]);
        assert_eq!(parse_claude_line(r#"{"type":"result","result":"done"}"#), vec![AgentEvent::Final("done".into())]);
        assert!(parse_claude_line("not json").is_empty());
    }
    #[test] fn streams_activity_and_final_answer() {
        let script = r#"echo '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"a/b.rs"}}]}}'; echo '{"type":"result","result":"all good"}'"#;
        let mut got = vec![]; let c = AtomicBool::new(false);
        let r = run_streaming(&sh(script, true), &std::env::temp_dir(), &env(), Duration::from_secs(10), &c, |e| got.push(e)).unwrap();
        assert_eq!(r, "all good"); assert_eq!(got[0], AgentEvent::Activity("Reading b.rs".into()));
    }
    #[test] fn plain_output_is_the_answer_and_failure_reports_stderr() {
        let c = AtomicBool::new(false);
        assert_eq!(run_streaming(&sh("echo one; echo two", false), &std::env::temp_dir(), &env(), Duration::from_secs(10), &c, |_| {}).unwrap().trim(), "one\ntwo");
        let e = run_streaming(&sh("echo oops >&2; exit 3", false), &std::env::temp_dir(), &env(), Duration::from_secs(10), &c, |_| {}).unwrap_err();
        assert!(matches!(e, RunError::Failed(m) if m.contains("oops")));
        assert!(matches!(run_streaming(&Launch { program: "definitely-missing-xyz".into(), ..Default::default() }, &std::env::temp_dir(), &env(), Duration::from_secs(1), &c, |_| {}), Err(RunError::Failed(_))));
    }
    #[test] fn cancel_and_timeout_kill_the_process() {
        let cancel = Arc::new(AtomicBool::new(false)); let c2 = cancel.clone();
        std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(300)); c2.store(true, Ordering::Relaxed); });
        let t = Instant::now();
        assert_eq!(run_streaming(&sh("sleep 30", false), &std::env::temp_dir(), &env(), Duration::from_secs(60), &cancel, |_| {}), Err(RunError::Cancelled)); assert!(t.elapsed() < Duration::from_secs(5));
        let c = AtomicBool::new(false);
        assert_eq!(run_streaming(&sh("sleep 30", false), &std::env::temp_dir(), &env(), Duration::from_millis(300), &c, |_| {}), Err(RunError::TimedOut(0)));
    }
    #[test] fn extra_env_reaches_agent_but_secrets_do_not() {
        let c = AtomicBool::new(false);
        let mut l = sh("echo \"$MAX_THINKING_TOKENS:${GITHUB_TOKEN:-none}\"", false); l.env = vec![("MAX_THINKING_TOKENS".into(), "4000".into())];
        let mut e = env(); e.insert("GITHUB_TOKEN".into(), "x".into()); let e = safety::minimal_env(&e);
        assert_eq!(run_streaming(&l, &std::env::temp_dir(), &e, Duration::from_secs(5), &c, |_| {}).unwrap().trim(), "4000:none");
    }
}
