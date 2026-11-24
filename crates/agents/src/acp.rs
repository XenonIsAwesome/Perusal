//! Minimal Agent Client Protocol client: JSON-RPC 2.0 over newline-delimited stdio.
use crate::safety::{Decision, Policy};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

pub struct Client { child: Child, stdin: ChildStdin, out: BufReader<std::process::ChildStdout>, next: u64 }

impl Client {
    /// Starts the agent in `cwd` (the checked-out MR tree) with a minimal environment.
    pub fn spawn(program: &str, args: &[String], cwd: &std::path::Path, env: &BTreeMap<String, String>) -> std::io::Result<Self> {
        let mut c = Command::new(program).args(args).current_dir(cwd).env_clear().envs(env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let stdin = c.stdin.take().unwrap(); let out = BufReader::new(c.stdout.take().unwrap());
        Ok(Client { child: c, stdin, out, next: 1 })
    }
    fn send(&mut self, v: &Value) -> std::io::Result<()> { writeln!(self.stdin, "{v}")?; self.stdin.flush() }
    fn read(&mut self) -> std::io::Result<Option<Value>> {
        let mut l = String::new();
        if self.out.read_line(&mut l)? == 0 { return Ok(None); }
        serde_json::from_str(&l).map(Some).map_err(std::io::Error::other)
    }
    /// Sends a request and returns its result, answering agent requests (permissions) via `policy`
    /// and collecting streamed text chunks into `text`.
    pub fn request(&mut self, method: &str, params: Value, policy: &mut Policy, text: &mut String) -> std::io::Result<Value> {
        let id = self.next; self.next += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        loop {
            let Some(m) = self.read()? else { return Err(std::io::Error::other("agent closed the connection")) };
            if m["id"] == json!(id) && m.get("method").is_none() {
                return if let Some(e) = m.get("error") { Err(std::io::Error::other(e.to_string())) } else { Ok(m["result"].clone()) };
            }
            match m["method"].as_str() {
                Some("session/update") => { if let Some(t) = m.pointer("/params/update/content/text").and_then(|x| x.as_str()) { text.push_str(t); } }
                Some("session/request_permission") => {
                    let kind = m.pointer("/params/toolCall/kind").and_then(|x| x.as_str()).unwrap_or("other");
                    let target = m.pointer("/params/toolCall/title").and_then(|x| x.as_str()).unwrap_or("");
                    let ok = policy.decide(kind, target) == Decision::Allow;
                    let rid = m["id"].clone();
                    self.send(&json!({"jsonrpc": "2.0", "id": rid, "result": {"outcome": {"outcome": "selected", "optionId": if ok { "allow" } else { "reject" }}}}))?;
                }
                _ => {}
            }
        }
    }
    pub fn initialize(&mut self, p: &mut Policy) -> std::io::Result<Value> { let mut t = String::new(); self.request("initialize", json!({"protocolVersion": 1, "clientCapabilities": {}}), p, &mut t) }
    pub fn new_session(&mut self, cwd: &str, p: &mut Policy) -> std::io::Result<String> {
        let mut t = String::new();
        let r = self.request("session/new", json!({"cwd": cwd, "mcpServers": []}), p, &mut t)?;
        r["sessionId"].as_str().map(String::from).ok_or_else(|| std::io::Error::other("no sessionId"))
    }
    /// Sends a prompt and returns the streamed answer text.
    pub fn prompt(&mut self, session: &str, text: &str, p: &mut Policy) -> std::io::Result<String> {
        let mut out = String::new();
        self.request("session/prompt", json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}), p, &mut out)?;
        Ok(out)
    }
    pub fn close(mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}
