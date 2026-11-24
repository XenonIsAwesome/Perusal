use perusal_providers::{github::GitHub, gitlab::GitLab, *};
use wiremock::matchers::{body_partial_json, method, path, header};
use wiremock::{Mock, MockServer, ResponseTemplate};
use serde_json::json;

fn gh(s: &MockServer) -> GitHub { GitHub { http: reqwest::Client::new(), api: s.uri(), token: "t".into() } }
fn gl(s: &MockServer) -> GitLab { GitLab { http: reqwest::Client::new(), api: s.uri(), token: "t".into() } }
fn ghref() -> MrRef { MrRef { kind: Kind::GitHub, host: "h".into(), project: "o/r".into(), number: 3 } }
fn glref() -> MrRef { MrRef { kind: Kind::GitLab, host: "h".into(), project: "g/p".into(), number: 4 } }
fn sub(v: Verdict) -> Submission { Submission { head_sha: "abc".into(), summary: "sum".into(), verdict: v, comments: vec![InlineComment { path: "a.rs".into(), start_line: 2, end_line: 4, body: "hm".into() }] } }

#[tokio::test] async fn github_info_and_ci() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/repos/o/r/pulls/3")).and(header("authorization", "Bearer t"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"title":"T","state":"closed","merged":true,"user":{"login":"u"},"head":{"sha":"abc"},"base":{"ref":"main"},"html_url":"https://x"}))).mount(&s).await;
    Mock::given(method("GET")).and(path("/repos/o/r/commits/abc/check-runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"check_runs":[{"status":"completed","conclusion":"success"},{"status":"in_progress","conclusion":null}]}))).mount(&s).await;
    let g = gh(&s);
    let i = g.mr_info(&ghref()).await.unwrap(); assert_eq!((i.state.as_str(), i.head_sha.as_str()), ("merged", "abc"));
    assert_eq!(g.ci(&ghref(), "abc").await.unwrap(), Ci::Running);
}
#[tokio::test] async fn github_submit_is_one_review_call() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/repos/o/r/pulls/3/reviews")).and(body_partial_json(json!({"event":"REQUEST_CHANGES","commit_id":"abc","comments":[{"path":"a.rs","start_line":2,"line":4}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&s).await;
    gh(&s).submit(&ghref(), &sub(Verdict::RequestChanges)).await.unwrap();
}
#[tokio::test] async fn github_threads_group_replies() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/repos/o/r/pulls/3/comments")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        {"id":1,"user":{"login":"a"},"body":"q","path":"x.rs","line":5},{"id":2,"in_reply_to_id":1,"user":{"login":"b"},"body":"a","path":"x.rs","line":5}]))).mount(&s).await;
    let t = gh(&s).threads(&ghref()).await.unwrap(); assert_eq!(t.len(), 1); assert_eq!(t[0].comments.len(), 2);
}
#[tokio::test] async fn errors_map() {
    let s = MockServer::start().await;
    Mock::given(path("/repos/o/r/pulls/3")).respond_with(ResponseTemplate::new(401)).mount(&s).await;
    assert!(matches!(gh(&s).mr_info(&ghref()).await, Err(Error::Unauthorized)));
    let s2 = MockServer::start().await;
    Mock::given(path("/repos/o/r/pulls/3")).respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30")).mount(&s2).await;
    assert!(matches!(gh(&s2).mr_info(&ghref()).await, Err(Error::RateLimited(Some(30)))));
    let dead = GitHub { http: reqwest::Client::new(), api: "http://127.0.0.1:1".into(), token: "t".into() };
    assert!(matches!(dead.mr_info(&ghref()).await, Err(Error::Offline(_))));
}
#[tokio::test] async fn gitlab_submit_publishes() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes")).respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":11}))).expect(2).mount(&s).await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes/bulk_publish")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/approve")).respond_with(ResponseTemplate::new(201)).expect(1).mount(&s).await;
    gl(&s).submit(&glref(), &sub(Verdict::Approve)).await.unwrap();
}
#[tokio::test] async fn gitlab_failed_submit_leaves_nothing_half_posted() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes")).respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":11}))).mount(&s).await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes/bulk_publish")).respond_with(ResponseTemplate::new(500)).mount(&s).await;
    Mock::given(method("DELETE")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes/11")).respond_with(ResponseTemplate::new(204)).expect(2).mount(&s).await;
    assert!(gl(&s).submit(&glref(), &sub(Verdict::Comment)).await.is_err());
}
#[tokio::test] async fn gitlab_ci_threads_and_degradation() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/projects/g%2Fp/merge_requests/4")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"head_pipeline":{"status":"success"}}))).mount(&s).await;
    Mock::given(method("GET")).and(path("/projects/g%2Fp/merge_requests/4/discussions")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        {"id":"d1","individual_note":false,"notes":[{"author":{"username":"a"},"body":"x","resolved":true,"position":{"new_path":"f.rs","new_line":7}}]},
        {"id":"d2","individual_note":true,"notes":[{"author":{"username":"a"},"body":"hi"}]}]))).mount(&s).await;
    let g = gl(&s);
    assert_eq!(g.ci(&glref(), "").await.unwrap(), Ci::Passed);
    let t = g.threads(&glref()).await.unwrap(); assert_eq!(t.len(), 1); assert!(t[0].resolved); assert_eq!(t[0].line, Some(7));
    assert!(matches!(g.set_viewed(&glref(), "f", true).await, Err(Error::Unsupported(_))));
    assert!(matches!(g.submit(&glref(), &sub(Verdict::RequestChanges)).await, Err(Error::Unsupported(_))));
}
#[tokio::test] async fn device_flow() {
    let s = MockServer::start().await;
    Mock::given(path("/login/device/code")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code":"dc","user_code":"UC","verification_uri":"https://v","interval":0}))).mount(&s).await;
    Mock::given(path("/login/oauth/access_token")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"tok"}))).mount(&s).await;
    let c = reqwest::Client::new();
    let d = auth::device_start(&c, &s.uri(), "cid").await.unwrap(); assert_eq!(d.user_code, "UC");
    assert_eq!(auth::device_poll(&c, &s.uri(), "cid", &d, 3).await.unwrap(), "tok");
}

#[tokio::test] async fn thread_parity_github() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/repos/o/r/pulls/3/comments/9/replies")).respond_with(ResponseTemplate::new(201)).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/repos/o/r/pulls/comments/9/reactions")).respond_with(ResponseTemplate::new(201)).expect(1).mount(&s).await;
    Mock::given(method("PATCH")).and(path("/repos/o/r/pulls/comments/9")).and(body_partial_json(json!({"body":"new"}))).respond_with(ResponseTemplate::new(200)).expect(1).mount(&s).await;
    Mock::given(method("DELETE")).and(path("/repos/o/r/pulls/comments/9")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/graphql")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{}}))).expect(1).mount(&s).await;
    Mock::given(method("GET")).and(path("/repos/o/r/pulls")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"number":3,"title":"t"}]))).mount(&s).await;
    let g = gh(&s);
    g.reply(&ghref(), "9", "hi").await.unwrap(); g.react(&ghref(), "9", "9", "+1").await.unwrap(); g.edit_comment(&ghref(), "9", "9", "new").await.unwrap();
    g.delete_comment(&ghref(), "9", "9").await.unwrap(); g.resolve(&ghref(), "PRRT_x", true).await.unwrap();
    assert_eq!(g.find_for_branch("o/r", "feat").await.unwrap(), Some(3));
}
#[tokio::test] async fn thread_parity_gitlab() {
    let s = MockServer::start().await; let b = "/projects/g%2Fp/merge_requests/4";
    Mock::given(method("POST")).and(path(format!("{b}/discussions/d1/notes"))).respond_with(ResponseTemplate::new(201)).expect(1).mount(&s).await;
    Mock::given(method("PUT")).and(path(format!("{b}/discussions/d1"))).and(body_partial_json(json!({"resolved":false}))).respond_with(ResponseTemplate::new(200)).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path(format!("{b}/notes/5/award_emoji"))).respond_with(ResponseTemplate::new(201)).expect(1).mount(&s).await;
    Mock::given(method("PUT")).and(path(format!("{b}/notes/5"))).respond_with(ResponseTemplate::new(200)).expect(1).mount(&s).await;
    Mock::given(method("DELETE")).and(path(format!("{b}/notes/5"))).respond_with(ResponseTemplate::new(204)).expect(1).mount(&s).await;
    Mock::given(method("GET")).and(path("/projects/g%2Fp/merge_requests")).respond_with(ResponseTemplate::new(200).set_body_json(json!([]))).mount(&s).await;
    let g = gl(&s);
    g.reply(&glref(), "d1", "x").await.unwrap(); g.resolve(&glref(), "d1", false).await.unwrap(); g.react(&glref(), "d1", "5", "thumbsup").await.unwrap();
    g.edit_comment(&glref(), "d1", "5", "y").await.unwrap(); g.delete_comment(&glref(), "d1", "5").await.unwrap();
    assert_eq!(g.find_for_branch("g/p", "feat").await.unwrap(), None);
}
