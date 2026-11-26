use perusal_providers::{github::GitHub, gitlab::GitLab, *};
use wiremock::matchers::{body_partial_json, method, path, header};
use wiremock::{Mock, MockServer, ResponseTemplate};
use serde_json::json;

fn gh(s: &MockServer) -> GitHub { GitHub { http: reqwest::Client::new(), api: s.uri(), token: "t".into() } }
fn gl(s: &MockServer) -> GitLab { GitLab { http: reqwest::Client::new(), api: s.uri(), token: "t".into() } }
fn ghref() -> MrRef { MrRef { kind: Kind::GitHub, host: "h".into(), project: "o/r".into(), number: 3 } }
fn glref() -> MrRef { MrRef { kind: Kind::GitLab, host: "h".into(), project: "g/p".into(), number: 4 } }
fn sub(v: Verdict) -> Submission { Submission { head_sha: "abc".into(), summary: "sum".into(), verdict: v, comments: vec![InlineComment { path: "a.rs".into(), start_line: 2, end_line: 4, body: "hm".into(), start_old: 2, end_old: 3 }], general: vec![] } }

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
    // general conversation on the PR is added as path-less threads
    Mock::given(method("GET")).and(path("/repos/o/r/issues/3/comments")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":9,"user":{"login":"c"},"body":"overall"}]))).mount(&s).await;
    let t = gh(&s).threads(&ghref()).await.unwrap(); assert_eq!(t.len(), 2); assert!(t[1].path.is_none() && t[1].comments[0] == ("c".to_string(), "overall".to_string()));
}
#[tokio::test] async fn mr_description_is_read() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/repos/o/r/pulls/3")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"title":"T","state":"open","user":{"login":"u"},"head":{"sha":"a","ref":"feat"},"base":{"ref":"main"},"html_url":"x","body":"Why this exists"}))).mount(&s).await;
    let i = gh(&s).mr_info(&ghref()).await.unwrap(); assert_eq!((i.description.as_str(), i.source_ref.as_str()), ("Why this exists", "feat"));
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
    Mock::given(method("GET")).and(path("/projects/g%2Fp/merge_requests/4")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"diff_refs":{"base_sha":"BASE","start_sha":"START","head_sha":"HEAD"}}))).mount(&s).await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes")).respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":11}))).expect(2).mount(&s).await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes/bulk_publish")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/approve")).respond_with(ResponseTemplate::new(201)).expect(1).mount(&s).await;
    gl(&s).submit(&glref(), &sub(Verdict::Approve)).await.unwrap();
    // inline positions carry the MR's real diff refs, otherwise GitLab posts a plain MR thread
    let posted: Vec<serde_json::Value> = s.received_requests().await.unwrap().iter().filter(|r| r.url.path().ends_with("/draft_notes")).map(|r| serde_json::from_slice(&r.body).unwrap()).collect();
    let pos = &posted[0]["position"];
    assert_eq!((pos["base_sha"].as_str(), pos["start_sha"].as_str(), pos["head_sha"].as_str(), pos["position_type"].as_str()), (Some("BASE"), Some("START"), Some("HEAD"), Some("text")));
    assert!(pos["new_line"].as_u64().is_some() && pos["old_path"].as_str().is_some());
    // a multi-line selection (lines 2-4) is sent as a range, not just its last line
    assert_eq!((pos["new_line"].as_u64(), pos["line_range"]["start"]["new_line"].as_u64(), pos["line_range"]["end"]["new_line"].as_u64()), (Some(4), Some(2), Some(4)));
    assert!(pos["line_range"]["start"]["line_code"].as_str().unwrap().ends_with("_2_2") && pos["line_range"]["end"]["line_code"].as_str().unwrap().ends_with("_3_4"));
}
#[tokio::test] async fn gitlab_failed_submit_leaves_nothing_half_posted() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/projects/g%2Fp/merge_requests/4")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"diff_refs":{"base_sha":"BASE","start_sha":"START","head_sha":"HEAD"}}))).mount(&s).await;
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
        {"id":"d2","individual_note":true,"notes":[{"author":{"username":"a"},"body":"hi"}]},
        {"id":"d3","individual_note":true,"notes":[{"author":{"username":"gitlab"},"body":"added 1 commit","system":true}]}]))).mount(&s).await;
    let g = gl(&s);
    assert_eq!(g.ci(&glref(), "").await.unwrap(), Ci::Passed);
    let t = g.threads(&glref()).await.unwrap(); assert_eq!(t.len(), 2, "inline thread plus the general note; system notes are skipped"); assert!(t[0].resolved); assert_eq!(t[0].line, Some(7));
    assert!(t[1].path.is_none() && t[1].comments[0].1 == "hi");
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

#[tokio::test] async fn empty_token_sends_no_authorization_header() {
    use wiremock::matchers::any;
    for gitlab in [false, true] {
        let s = MockServer::start().await;
        Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_json(json!({"title":"T","state":"open","user":{"login":"u"},"author":{"username":"u"},"head":{"sha":"abc"},"sha":"abc","base":{"ref":"main"},"target_branch":"main","html_url":"x","web_url":"x"}))).mount(&s).await;
        if gitlab { let g = GitLab { http: reqwest::Client::new(), api: s.uri(), token: String::new() }; g.mr_info(&glref()).await.unwrap(); }
        else { let g = GitHub { http: reqwest::Client::new(), api: s.uri(), token: String::new() }; g.mr_info(&ghref()).await.unwrap(); }
        let reqs = s.received_requests().await.unwrap();
        assert!(reqs.iter().all(|r| !r.headers.contains_key("authorization") && !r.headers.contains_key("private-token")), "a public request must be anonymous");
    }
}

#[tokio::test] async fn my_reviews_github_and_gitlab() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/search/issues")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[
        {"html_url":"https://github.com/o/r/pull/7","title":"A","number":7,"user":{"login":"bob"},"state":"open","pull_request":{"merged_at":null},"repository_url":"https://api.github.com/repos/o/r","updated_at":"2026-01-01T00:00:00Z"},
        {"html_url":"https://github.com/o/r/pull/6","title":"B","number":6,"user":{"login":"al"},"state":"closed","pull_request":{"merged_at":"2026-01-02"},"repository_url":"https://api.github.com/repos/o/r","updated_at":"x"}]}))).mount(&s).await;
    let g = GitHub { http: reqwest::Client::new(), api: s.uri(), token: "t".into() };
    let l = g.my_reviews("github.com", Role::ReviewRequested, StateFilter::All).await.unwrap();
    assert_eq!((l[0].project.as_str(), l[0].state.as_str(), l[0].number), ("o/r", "open", 7)); assert_eq!(l[1].state, "merged");
    assert!(matches!(GitHub { http: reqwest::Client::new(), api: s.uri(), token: String::new() }.my_reviews("h", Role::Authored, StateFilter::Open).await, Err(Error::Unauthorized)));
    let s2 = MockServer::start().await;
    Mock::given(method("GET")).and(path("/user")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"username":"me","login":"me"}))).mount(&s2).await;
    Mock::given(method("GET")).and(path("/merge_requests")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"web_url":"http://gl.corp/grp/sub/proj/-/merge_requests/3","title":"C","iid":3,"author":{"username":"zed"},"state":"opened","updated_at":"u"}]))).mount(&s2).await;
    let l = GitLab { http: reqwest::Client::new(), api: s2.uri(), token: "t".into() }.my_reviews("gl.corp", Role::ReviewRequested, StateFilter::Open).await.unwrap();
    assert_eq!((l[0].project.as_str(), l[0].state.as_str(), l[0].number), ("grp/sub/proj", "open", 3));
    assert_eq!(l[0].url, "https://gl.corp/grp/sub/proj/-/merge_requests/3", "http web_url from the instance is rewritten to https");
    assert_eq!(ci_page(&Kind::GitHub, "https://github.com/o/r/pull/7"), "https://github.com/o/r/pull/7/checks");
}

#[tokio::test] async fn github_resolved_threads_paged_and_failure_tolerant() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/repos/o/r/pulls/3/comments")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        {"id":1,"user":{"login":"a"},"body":"q","path":"x.rs","line":5},{"id":2,"user":{"login":"a"},"body":"r","path":"x.rs","line":6},{"id":3,"user":{"login":"a"},"body":"s","path":"x.rs","line":7}]))).mount(&s).await;
    // page 2 is only served when the cursor from page 1 is sent back
    Mock::given(method("POST")).and(path("/graphql")).and(wiremock::matchers::body_string_contains("\"c\":\"cur1\"")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"repository":{"pullRequest":{"reviewThreads":{
        "pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"isResolved":true,"comments":{"nodes":[{"databaseId":3}]}}]}}}}}))).mount(&s).await;
    Mock::given(method("POST")).and(path("/graphql")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"repository":{"pullRequest":{"reviewThreads":{
        "pageInfo":{"hasNextPage":true,"endCursor":"cur1"},"nodes":[{"isResolved":true,"comments":{"nodes":[{"databaseId":1}]}},{"isResolved":false,"comments":{"nodes":[{"databaseId":2}]}}]}}}}}))).mount(&s).await;
    let t = gh(&s).threads(&ghref()).await.unwrap();
    assert_eq!(t.iter().map(|x| x.resolved).collect::<Vec<_>>(), vec![true, false, true]);
    // a failing GraphQL endpoint keeps the threads, just unresolved
    let s2 = MockServer::start().await;
    Mock::given(method("GET")).and(path("/repos/o/r/pulls/3/comments")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":1,"user":{"login":"a"},"body":"q","path":"x.rs","line":5}]))).mount(&s2).await;
    Mock::given(method("POST")).and(path("/graphql")).respond_with(ResponseTemplate::new(500)).mount(&s2).await;
    let t = gh(&s2).threads(&ghref()).await.unwrap(); assert_eq!(t.len(), 1); assert!(!t[0].resolved);
}
#[tokio::test] async fn gitlab_thread_resolution_rules() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/projects/g%2Fp/merge_requests/4/discussions")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        {"id":"all","notes":[{"author":{"username":"a"},"body":"x","resolvable":true,"resolved":true},{"author":{"username":"b"},"body":"y","resolvable":true,"resolved":true}]},
        {"id":"mixed","notes":[{"author":{"username":"a"},"body":"x","resolvable":true,"resolved":true},{"author":{"username":"b"},"body":"y","resolvable":true,"resolved":false}]},
        {"id":"plain","notes":[{"author":{"username":"a"},"body":"x","resolvable":false,"resolved":false}]},
        {"id":"sys","notes":[{"author":{"username":"gitlab"},"body":"merged","system":true,"resolvable":true,"resolved":true}]}]))).mount(&s).await;
    let t = gl(&s).threads(&glref()).await.unwrap();
    assert_eq!(t.len(), 3, "the system-note-only discussion is dropped");
    assert_eq!(t.iter().map(|x| (x.id.as_str(), x.resolved)).collect::<Vec<_>>(), vec![("all", true), ("mixed", false), ("plain", false)]);
}
#[tokio::test] async fn github_general_comments_are_threads() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/repos/o/r/issues/3/comments")).and(body_partial_json(json!({"body":"overall"}))).respond_with(ResponseTemplate::new(201).set_body_json(json!({}))).expect(1).mount(&s).await;
    // only general comments: no empty review is created
    let only = Submission { head_sha: "abc".into(), summary: String::new(), verdict: Verdict::Comment, comments: vec![], general: vec!["overall".into()] };
    gh(&s).submit(&ghref(), &only).await.unwrap();
}
#[tokio::test] async fn gitlab_general_comments_are_unpositioned_draft_notes() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/projects/g%2Fp/merge_requests/4")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).mount(&s).await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes")).and(body_partial_json(json!({"note":"overall"}))).respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":5}))).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/projects/g%2Fp/merge_requests/4/draft_notes/bulk_publish")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&s).await;
    let only = Submission { head_sha: "abc".into(), summary: String::new(), verdict: Verdict::Comment, comments: vec![], general: vec!["overall".into()] };
    gl(&s).submit(&glref(), &only).await.unwrap();
}
#[tokio::test] async fn github_my_reviews_carries_people_labels_and_text() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/search/issues")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[{"html_url":"https://x/o/r/pull/1","title":"T","number":1,"state":"open","repository_url":"https://api/repos/o/r","updated_at":"2024","user":{"login":"al","avatar_url":"https://a/al.png"},
        "assignees":[{"login":"bo","avatar_url":"https://a/bo.png"}],"labels":[{"name":"bug"}],"body":"overview text","draft":true}]}))).mount(&s).await;
    let l = gh(&s).my_reviews("github.com", Role::Authored, StateFilter::Open).await.unwrap();
    assert_eq!((l[0].author_avatar.as_str(), l[0].assignees[0].login.as_str(), l[0].labels[0].as_str(), l[0].description.as_str(), l[0].draft), ("https://a/al.png", "bo", "bug", "overview text", true));
}
#[tokio::test] async fn gitlab_threads_carry_avatars() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/projects/g%2Fp/merge_requests/4/discussions")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":"d","notes":[{"system":false,"body":"hi","author":{"username":"u","avatar_url":"https://a/u.png"}}]}]))).mount(&s).await;
    let t = gl(&s).threads(&glref()).await.unwrap(); assert_eq!(t[0].avatars, vec!["https://a/u.png".to_string()]);
}
#[tokio::test] async fn github_reviewable_lists_pulls_of_repos_you_can_push_to() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/user/repos")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        {"full_name":"o/rw","permissions":{"push":true}}, {"full_name":"o/ro","permissions":{"push":false}}, {"full_name":"o/old","archived":true,"permissions":{"push":true}}]))).mount(&s).await;
    Mock::given(method("GET")).and(path("/repos/o/rw/pulls")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        {"html_url":"https://x/o/rw/pull/1","title":"A","number":1,"state":"open","merged_at":null,"updated_at":"2026","user":{"login":"al"},"assignees":[],"labels":[{"name":"bug"}],"body":"b","draft":false,"requested_reviewers":[{"login":"me"}]}]))).expect(1).mount(&s).await;
    let l = gh(&s).my_reviews("github.com", Role::Reviewable, StateFilter::Open).await.unwrap();
    assert_eq!(l.len(), 1); assert_eq!((l[0].project.as_str(), l[0].reviewers[0].login.as_str(), l[0].labels[0].as_str()), ("o/rw", "me", "bug"));
}
#[tokio::test] async fn gitlab_reviewable_lists_mrs_of_projects_with_developer_access() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/projects")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":7},{"id":8}]))).mount(&s).await;
    for id in [7, 8] {
        Mock::given(method("GET")).and(path(format!("/projects/{id}/merge_requests"))).respond_with(ResponseTemplate::new(if id == 8 { 403 } else { 200 }).set_body_json(json!([
            {"web_url":"https://h/g/p/-/merge_requests/3","iid":3,"title":"T","state":"opened","updated_at":"2026","author":{"username":"u"},"assignees":[],"reviewers":[],"labels":["x"],"description":"","draft":false}]))).mount(&s).await;
    }
    let l = gl(&s).my_reviews("h", Role::Reviewable, StateFilter::Open).await.unwrap();
    assert_eq!(l.len(), 1); assert_eq!((l[0].project.as_str(), l[0].number), ("g/p", 3));
}
#[tokio::test] async fn github_reviewable_errors_when_every_repo_fails() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/user/repos")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"full_name":"o/a","permissions":{"push":true}},{"full_name":"o/b","permissions":{"push":true}}]))).mount(&s).await;
    Mock::given(method("GET")).and(path("/repos/o/a/pulls")).respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30")).mount(&s).await;
    Mock::given(method("GET")).and(path("/repos/o/b/pulls")).respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30")).mount(&s).await;
    assert!(matches!(gh(&s).my_reviews("github.com", Role::Reviewable, StateFilter::Open).await, Err(Error::RateLimited(Some(30)))));
}
#[tokio::test] async fn github_reviewable_keeps_results_when_only_some_repos_fail() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/user/repos")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"full_name":"o/a","permissions":{"push":true}},{"full_name":"o/b","permissions":{"push":true}}]))).mount(&s).await;
    Mock::given(method("GET")).and(path("/repos/o/a/pulls")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"html_url":"https://x/o/a/pull/1","title":"A","number":1,"state":"open","merged_at":null,"updated_at":"2026","user":{"login":"u"},"assignees":[],"labels":[],"body":"","draft":false}]))).mount(&s).await;
    Mock::given(method("GET")).and(path("/repos/o/b/pulls")).respond_with(ResponseTemplate::new(500)).mount(&s).await;
    assert_eq!(gh(&s).my_reviews("github.com", Role::Reviewable, StateFilter::Open).await.unwrap().len(), 1);
}
#[tokio::test] async fn gitlab_reviewable_errors_when_every_project_fails() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/projects")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":7},{"id":8}]))).mount(&s).await;
    for id in [7, 8] { Mock::given(method("GET")).and(path(format!("/projects/{id}/merge_requests"))).respond_with(ResponseTemplate::new(401)).mount(&s).await; }
    assert!(matches!(gl(&s).my_reviews("h", Role::Reviewable, StateFilter::Open).await, Err(Error::Unauthorized)));
}
