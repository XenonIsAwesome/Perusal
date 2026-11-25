use perusal_agents::{acp::Client, parse_review, safety::*};
use std::collections::BTreeMap;

fn start(env: &BTreeMap<String, String>) -> Client { Client::spawn(env!("CARGO_BIN_EXE_fake-agent"), &[], &std::env::temp_dir(), env).unwrap() }
fn session(c: &mut Client, p: &mut Policy) -> String { c.initialize(p).unwrap(); c.new_session("/tmp", p).unwrap() }

#[test] fn chat_roundtrip() {
    let mut p = Policy::default(); let mut c = start(&BTreeMap::new()); let s = session(&mut c, &mut p);
    assert_eq!(c.prompt(&s, "what does this do?", &mut p).unwrap(), "echo: what does this do?"); c.close();
}
#[test] fn full_review_lands_as_ai_drafts() {
    let mut p = Policy::default(); let mut c = start(&BTreeMap::new()); let s = session(&mut c, &mut p);
    let out = c.prompt(&s, "REVIEW please", &mut p).unwrap(); c.close();
    let v = parse_review(&out).unwrap(); assert_eq!(v.len(), 2);
    let st = perusal_core::store::Store::memory().unwrap(); let r = st.open_review("k", "t", "h", 2).unwrap();
    for c in &v { st.add_draft(r, Some(&c.file), Some((c.start_line, c.end_line)), &perusal_agents::draft_body(c), true).unwrap(); }
    let d = st.drafts(r).unwrap(); assert!(d.iter().all(|x| x.ai) && d[0].body == "[major] off by one");
}
#[test] fn permissions_enforced_over_acp() {
    let mut p = Policy::default(); let mut c = start(&BTreeMap::new()); let s = session(&mut c, &mut p);
    assert_eq!(c.prompt(&s, "TOOL:edit:a.rs", &mut p).unwrap(), "permission:reject");
    assert_eq!(c.prompt(&s, "TOOL:fetch:https://docs.rs", &mut p).unwrap(), "permission:allow");
    assert_eq!(c.prompt(&s, "TOOL:read:repo:evil/x", &mut p).unwrap(), "permission:reject");
    assert!(p.log.contains(&Access::WebFetch("https://docs.rs".into()))); c.close();
}
#[test] fn secrets_never_reach_agent() {
    let parent: BTreeMap<String, String> = [("PATH", std::env::var("PATH").unwrap()), ("GITHUB_TOKEN", "abc".into()), ("PERUSAL_TOKEN", "abc".into())].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let mut p = Policy::default(); let mut c = start(&minimal_env(&parent)); let s = session(&mut c, &mut p);
    assert_eq!(c.prompt(&s, "ENV", &mut p).unwrap(), "secrets_in_env=0"); c.close();
}
