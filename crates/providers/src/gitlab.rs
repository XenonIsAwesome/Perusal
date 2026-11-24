use crate::*;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};

pub struct GitLab { pub http: reqwest::Client, pub api: String, pub token: String }

impl GitLab {
    pub fn api_base(host: &str) -> String { format!("https://{host}/api/v4") }
    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder { self.http.request(m, format!("{}{path}", self.api)).header("PRIVATE-TOKEN", &self.token) }
    fn mr(r: &MrRef) -> String { format!("/projects/{}/merge_requests/{}", utf8_percent_encode(&r.project, NON_ALPHANUMERIC), r.number) }
}

#[async_trait::async_trait]
impl Provider for GitLab {
    async fn mr_info(&self, r: &MrRef) -> Result<MrInfo> {
        let v: Value = check(self.req(reqwest::Method::GET, &Self::mr(r)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let s = |p: &str| v.pointer(p).and_then(|x| x.as_str()).unwrap_or("").to_string();
        Ok(MrInfo { title: s("/title"), state: s("/state"), author: s("/author/username"), head_sha: s("/sha"), base_ref: s("/target_branch"), web_url: s("/web_url") })
    }
    async fn threads(&self, r: &MrRef) -> Result<Vec<Thread>> {
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("{}/discussions", Self::mr(r))).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v.iter().filter(|d| d["individual_note"].as_bool() != Some(true)).map(|d| {
            let notes = d["notes"].as_array().cloned().unwrap_or_default();
            let first = notes.first().cloned().unwrap_or(Value::Null);
            Thread { id: d["id"].as_str().unwrap_or("").into(), path: first.pointer("/position/new_path").and_then(|x| x.as_str()).map(String::from),
                line: first.pointer("/position/new_line").and_then(|x| x.as_u64()).map(|l| l as u32), resolved: notes.iter().any(|n| n["resolved"].as_bool() == Some(true)),
                comments: notes.iter().map(|n| (n["author"]["username"].as_str().unwrap_or("").into(), n["body"].as_str().unwrap_or("").into())).collect() }
        }).collect())
    }
    /// Draft notes then bulk publish. If any step fails, drafts already created are deleted: nothing half-posted.
    async fn submit(&self, r: &MrRef, s: &Submission) -> Result<()> {
        if s.verdict == Verdict::RequestChanges { return Err(Error::Unsupported("request changes")); }
        let base = Self::mr(r);
        let mut created: Vec<u64> = Vec::new();
        let mut bodies: Vec<Value> = s.comments.iter().map(|c| json!({"note": c.body, "position": {"position_type": "text", "new_path": c.path, "new_line": c.end_line, "base_sha": null, "head_sha": s.head_sha, "start_sha": null}})).collect();
        if !s.summary.is_empty() { bodies.push(json!({"note": s.summary})); }
        let res: Result<()> = async {
            for b in &bodies {
                let v: Value = check(self.req(reqwest::Method::POST, &format!("{base}/draft_notes")).json(b).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
                if let Some(id) = v["id"].as_u64() { created.push(id); }
            }
            check(self.req(reqwest::Method::POST, &format!("{base}/draft_notes/bulk_publish")).send().await).await?;
            if s.verdict == Verdict::Approve { check(self.req(reqwest::Method::POST, &format!("{base}/approve")).send().await).await?; }
            Ok(())
        }.await;
        if res.is_err() { for id in created { let _ = self.req(reqwest::Method::DELETE, &format!("{base}/draft_notes/{id}")).send().await; } }
        res
    }
    async fn ci(&self, r: &MrRef, _sha: &str) -> Result<Ci> {
        let v: Value = check(self.req(reqwest::Method::GET, &Self::mr(r)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(match v.pointer("/head_pipeline/status").and_then(|x| x.as_str()) {
            None => Ci::None, Some("running") => Ci::Running, Some("pending" | "created" | "waiting_for_resource" | "preparing" | "scheduled") => Ci::Pending,
            Some("success") => Ci::Passed, Some("failed") => Ci::Failed, Some(_) => Ci::Canceled })
    }
    async fn request_review(&self, r: &MrRef, users: &[String]) -> Result<()> {
        let mut ids = Vec::new();
        for u in users {
            let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/users?username={u}")).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
            if let Some(id) = v.first().and_then(|x| x["id"].as_u64()) { ids.push(id); }
        }
        check(self.req(reqwest::Method::PUT, &Self::mr(r)).json(&json!({"reviewer_ids": ids})).send().await).await.map(|_| ())
    }
    /// GitLab's API has no per-file viewed state; marks stay local (see DECISIONS.md).
    async fn set_viewed(&self, _: &MrRef, _: &str, _: bool) -> Result<()> { Err(Error::Unsupported("viewed sync")) }
    async fn list_open(&self, project: &str) -> Result<Vec<(u64, String)>> {
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/projects/{}/merge_requests?state=opened", utf8_percent_encode(project, NON_ALPHANUMERIC))).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v.iter().map(|p| (p["iid"].as_u64().unwrap_or(0), p["title"].as_str().unwrap_or("").to_string())).collect())
    }
    async fn reply(&self, r: &MrRef, thread: &str, body: &str) -> Result<()> {
        check(self.req(reqwest::Method::POST, &format!("{}/discussions/{thread}/notes", Self::mr(r))).json(&json!({"body": body})).send().await).await.map(|_| ())
    }
    async fn resolve(&self, r: &MrRef, thread: &str, resolved: bool) -> Result<()> {
        check(self.req(reqwest::Method::PUT, &format!("{}/discussions/{thread}", Self::mr(r))).json(&json!({"resolved": resolved})).send().await).await.map(|_| ())
    }
    async fn react(&self, r: &MrRef, _thread: &str, comment: &str, emoji: &str) -> Result<()> {
        check(self.req(reqwest::Method::POST, &format!("{}/notes/{comment}/award_emoji", Self::mr(r))).json(&json!({"name": emoji})).send().await).await.map(|_| ())
    }
    async fn edit_comment(&self, r: &MrRef, _thread: &str, comment: &str, body: &str) -> Result<()> {
        check(self.req(reqwest::Method::PUT, &format!("{}/notes/{comment}", Self::mr(r))).json(&json!({"body": body})).send().await).await.map(|_| ())
    }
    async fn delete_comment(&self, r: &MrRef, _thread: &str, comment: &str) -> Result<()> {
        check(self.req(reqwest::Method::DELETE, &format!("{}/notes/{comment}", Self::mr(r))).send().await).await.map(|_| ())
    }
    async fn find_for_branch(&self, project: &str, branch: &str) -> Result<Option<u64>> {
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/projects/{}/merge_requests?state=opened&source_branch={}", utf8_percent_encode(project, NON_ALPHANUMERIC), utf8_percent_encode(branch, NON_ALPHANUMERIC))).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v.first().and_then(|p| p["iid"].as_u64()))
    }
}
