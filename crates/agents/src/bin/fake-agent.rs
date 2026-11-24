//! Test double: a tiny ACP agent. Prompts containing "REVIEW" return a JSON review; "TOOL:<kind>:<title>" asks permission first.
use std::io::{BufRead, Write};
use serde_json::{json, Value};

fn main() {
    let stdin = std::io::stdin(); let mut out = std::io::stdout();
    let mut emit = |v: Value| { writeln!(out, "{v}").unwrap(); out.flush().unwrap(); };
    let secrets = std::env::vars().filter(|(k, _)| k.contains("TOKEN") || k.contains("SECRET")).count();
    let mut lines = stdin.lock().lines();
    while let Some(Ok(l)) = lines.next() {
        let m: Value = serde_json::from_str(&l).unwrap(); let id = m["id"].clone();
        match m["method"].as_str() {
            Some("initialize") => emit(json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":1}})),
            Some("session/new") => emit(json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":"s1"}})),
            Some("session/prompt") => {
                let p = m.pointer("/params/prompt/0/text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let chunk = |t: &str| json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":t}}}});
                if let Some(rest) = p.strip_prefix("TOOL:") {
                    let (kind, title) = rest.split_once(':').unwrap_or((rest, ""));
                    emit(json!({"jsonrpc":"2.0","id":900,"method":"session/request_permission","params":{"sessionId":"s1","toolCall":{"kind":kind,"title":title}}}));
                    let resp: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                    emit(chunk(&format!("permission:{}", resp.pointer("/result/outcome/optionId").and_then(|x| x.as_str()).unwrap_or("?"))));
                } else if p.contains("REVIEW") {
                    emit(chunk("```json\n[{\"file\":\"a.rs\",\"start_line\":2,\"end_line\":2,\"severity\":\"major\",\"body\":\"off by one\"},"));
                    emit(chunk("{\"file\":\"b.rs\",\"start_line\":1,\"end_line\":3,\"severity\":\"nit\",\"body\":\"naming\"}]\n```"));
                } else if p.contains("ENV") {
                    emit(chunk(&format!("secrets_in_env={secrets}")));
                } else { emit(chunk(&format!("echo: {p}"))); }
                emit(json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"end_turn"}}));
            }
            _ => emit(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"nope"}})),
        }
    }
}
