use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn opens_mr_from_url_against_mock() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/repos/o/r/pulls/5")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(
        {"title":"Add x","state":"open","user":{"login":"bob"},"head":{"sha":"deadbeef"},"base":{"ref":"main"},"html_url":"u"}))).mount(&s).await;
    Mock::given(method("GET")).and(path("/repos/o/r/commits/deadbeef/check-runs")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"check_runs":[]}))).mount(&s).await;
    let api = s.uri();
    let out = tokio::task::spawn_blocking(move || std::process::Command::new(env!("CARGO_BIN_EXE_perusal-cli"))
        .args(["mr", "https://github.com/o/r/pull/5", "--api", &api]).output().unwrap()).await.unwrap();
    let t = String::from_utf8_lossy(&out.stdout);
    assert!(t.contains("Add x [open] by bob @ deadbeef") && t.contains("ci: None"), "{t} {}", String::from_utf8_lossy(&out.stderr));
}
