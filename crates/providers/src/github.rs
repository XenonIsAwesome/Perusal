use crate::*;
use serde_json::{json, Value};

pub struct GitHub { pub http: reqwest::Client, pub api: String, pub token: String }

impl GitHub {
    /// github.com uses api.github.com; Enterprise Server serves the API at https://host/api/v3.
    pub fn api_base(host: &str) -> String { if host == "github.com" { "https://api.github.com".into() } else { format!("https://{host}/api/v3") } }
    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http.request(m, format!("{}{path}", self.api)).bearer_auth(&self.token).header("accept", "application/vnd.github+json")
    }
}

#[async_trait::async_trait]
impl Provider for GitHub {
    async fn mr_info(&self, r: &MrRef) -> Result<MrInfo> {
        let v: Value = check(self.req(reqwest::Method::GET, &format!("/repos/{}/pulls/{}", r.project, r.number)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let s = |p: &str| v.pointer(p).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let state = if v["merged"].as_bool() == Some(true) { "merged".into() } else { s("/state") };
        Ok(MrInfo { title: s("/title"), state, author: s("/user/login"), head_sha: s("/head/sha"), base_ref: s("/base/ref"), web_url: s("/html_url") })
    }
    async fn threads(&self, r: &MrRef) -> Result<Vec<Thread>> {
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/repos/{}/pulls/{}/comments", r.project, r.number)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let mut out: Vec<Thread> = Vec::new();
        for c in v {
            let root = c["in_reply_to_id"].as_u64().map(|x| x.to_string()).unwrap_or(c["id"].to_string());
            let who = c["user"]["login"].as_str().unwrap_or("").to_string(); let body = c["body"].as_str().unwrap_or("").to_string();
            if let Some(t) = out.iter_mut().find(|t| t.id == root) { t.comments.push((who, body)); }
            else { out.push(Thread { id: root, path: c["path"].as_str().map(String::from), line: c["line"].as_u64().map(|l| l as u32), resolved: false, comments: vec![(who, body)] }); }
        }
        Ok(out)
    }
    /// One review call carrying all comments, so a failure leaves nothing posted.
    async fn submit(&self, r: &MrRef, s: &Submission) -> Result<()> {
        let event = match s.verdict { Verdict::Approve => "APPROVE", Verdict::RequestChanges => "REQUEST_CHANGES", Verdict::Comment => "COMMENT" };
        let comments: Vec<Value> = s.comments.iter().map(|c| if c.start_line != c.end_line {
            json!({"path": c.path, "start_line": c.start_line, "line": c.end_line, "side": "RIGHT", "body": c.body})
        } else { json!({"path": c.path, "line": c.end_line, "side": "RIGHT", "body": c.body}) }).collect();
        let body = json!({"commit_id": s.head_sha, "body": s.summary, "event": event, "comments": comments});
        check(self.req(reqwest::Method::POST, &format!("/repos/{}/pulls/{}/reviews", r.project, r.number)).json(&body).send().await).await.map(|_| ())
    }
    async fn ci(&self, r: &MrRef, sha: &str) -> Result<Ci> {
        let v: Value = check(self.req(reqwest::Method::GET, &format!("/repos/{}/commits/{sha}/check-runs", r.project)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let runs = v["check_runs"].as_array().cloned().unwrap_or_default();
        if runs.is_empty() { return Ok(Ci::None); }
        let st = |x: &Value| (x["status"].as_str().unwrap_or("").to_string(), x["conclusion"].as_str().unwrap_or("").to_string());
        let all: Vec<_> = runs.iter().map(st).collect();
        if all.iter().any(|(_, c)| c == "failure" || c == "timed_out" || c == "action_required") { return Ok(Ci::Failed); }
        if all.iter().any(|(s, _)| s == "in_progress") { return Ok(Ci::Running); }
        if all.iter().any(|(s, _)| s == "queued" || s == "pending") { return Ok(Ci::Pending); }
        if all.iter().all(|(_, c)| c == "cancelled" || c == "skipped") { return Ok(Ci::Canceled); }
        Ok(Ci::Passed)
    }
    async fn request_review(&self, r: &MrRef, users: &[String]) -> Result<()> {
        check(self.req(reqwest::Method::POST, &format!("/repos/{}/pulls/{}/requested_reviewers", r.project, r.number)).json(&json!({"reviewers": users})).send().await).await.map(|_| ())
    }
    async fn set_viewed(&self, r: &MrRef, path: &str, viewed: bool) -> Result<()> {
        // GraphQL: needs the PR node id first.
        let gql = if self.api.ends_with("/api/v3") { self.api.replace("/api/v3", "/api/graphql") } else { format!("{}/graphql", self.api) };
        let q = json!({"query": "query($o:String!,$r:String!,$n:Int!){repository(owner:$o,name:$r){pullRequest(number:$n){id}}}",
            "variables": {"o": r.project.split('/').next(), "r": r.project.split('/').nth(1), "n": r.number}});
        let v: Value = check(self.http.post(&gql).bearer_auth(&self.token).json(&q).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let id = v.pointer("/data/repository/pullRequest/id").and_then(|x| x.as_str()).ok_or(Error::Http(0, "no PR id".into()))?.to_string();
        let m = if viewed { "markFileAsViewed" } else { "unmarkFileAsViewed" };
        let q = json!({"query": format!("mutation($i:ID!,$p:String!){{{m}(input:{{pullRequestId:$i,path:$p}}){{clientMutationId}}}}"), "variables": {"i": id, "p": path}});
        check(self.http.post(&gql).bearer_auth(&self.token).json(&q).send().await).await.map(|_| ())
    }
    async fn list_open(&self, project: &str) -> Result<Vec<(u64, String)>> {
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/repos/{project}/pulls?state=open")).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v.iter().map(|p| (p["number"].as_u64().unwrap_or(0), p["title"].as_str().unwrap_or("").to_string())).collect())
    }
    async fn reply(&self, r: &MrRef, thread: &str, body: &str) -> Result<()> {
        check(self.req(reqwest::Method::POST, &format!("/repos/{}/pulls/{}/comments/{thread}/replies", r.project, r.number)).json(&json!({"body": body})).send().await).await.map(|_| ())
    }
    async fn resolve(&self, _r: &MrRef, thread: &str, resolved: bool) -> Result<()> {
        // `thread` is the GraphQL review-thread node id here.
        let gql = if self.api.ends_with("/api/v3") { self.api.replace("/api/v3", "/api/graphql") } else { format!("{}/graphql", self.api) };
        let m = if resolved { "resolveReviewThread" } else { "unresolveReviewThread" };
        let q = json!({"query": format!("mutation($t:ID!){{{m}(input:{{threadId:$t}}){{thread{{id}}}}}}"), "variables": {"t": thread}});
        check(self.http.post(&gql).bearer_auth(&self.token).json(&q).send().await).await.map(|_| ())
    }
    async fn react(&self, r: &MrRef, _thread: &str, comment: &str, emoji: &str) -> Result<()> {
        check(self.req(reqwest::Method::POST, &format!("/repos/{}/pulls/comments/{comment}/reactions", r.project)).json(&json!({"content": emoji})).send().await).await.map(|_| ())
    }
    async fn edit_comment(&self, r: &MrRef, _thread: &str, comment: &str, body: &str) -> Result<()> {
        check(self.req(reqwest::Method::PATCH, &format!("/repos/{}/pulls/comments/{comment}", r.project)).json(&json!({"body": body})).send().await).await.map(|_| ())
    }
    async fn delete_comment(&self, r: &MrRef, _thread: &str, comment: &str) -> Result<()> {
        check(self.req(reqwest::Method::DELETE, &format!("/repos/{}/pulls/comments/{comment}", r.project)).send().await).await.map(|_| ())
    }
    async fn find_for_branch(&self, project: &str, branch: &str) -> Result<Option<u64>> {
        let owner = project.split('/').next().unwrap_or("");
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/repos/{project}/pulls?state=open&head={owner}:{branch}")).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v.first().and_then(|p| p["number"].as_u64()))
    }
}
