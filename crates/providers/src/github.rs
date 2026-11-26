use crate::*;
use serde_json::{json, Value};

pub struct GitHub { pub http: reqwest::Client, pub api: String, pub token: String }

impl GitHub {
    /// github.com uses api.github.com; Enterprise Server serves the API at https://host/api/v3.
    pub fn api_base(host: &str) -> String { if host == "github.com" { "https://api.github.com".into() } else { format!("https://{host}/api/v3") } }
    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let r = self.http.request(m, format!("{}{path}", self.api)).header("accept", "application/vnd.github+json");
        // an empty token must not send `Authorization: Bearer ` (GitHub answers 401 to it, even for public repositories)
        if self.token.is_empty() { r } else { r.bearer_auth(&self.token) }
    }
}

impl GitHub {
    fn graphql_url(&self) -> String { if self.api.ends_with("/api/v3") { self.api.replace("/api/v3", "/api/graphql") } else { format!("{}/graphql", self.api) } }
    /// root comment id -> (review thread node id, is resolved)
    async fn graphql_threads(&self, r: &MrRef) -> Result<std::collections::HashMap<String, (String, bool)>> {
        let (owner, name) = r.project.split_once('/').unwrap_or((r.project.as_str(), ""));
        let q = json!({"query": "query($o:String!,$n:String!,$p:Int!){repository(owner:$o,name:$n){pullRequest(number:$p){reviewThreads(first:100){nodes{id isResolved comments(first:1){nodes{databaseId}}}}}}}",
            "variables": {"o": owner, "n": name, "p": r.number}});
        let v: Value = check(self.http.post(self.graphql_url()).bearer_auth(&self.token).json(&q).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let mut m = std::collections::HashMap::new();
        for n in v.pointer("/data/repository/pullRequest/reviewThreads/nodes").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
            if let (Some(id), Some(db)) = (n["id"].as_str(), n.pointer("/comments/nodes/0/databaseId").and_then(|x| x.as_u64())) { m.insert(db.to_string(), (id.to_string(), n["isResolved"].as_bool() == Some(true))); }
        }
        Ok(m)
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
            let cid = c["id"].to_string();
            if let Some(t) = out.iter_mut().find(|t| t.id == root) { t.comments.push((who, body)); t.comment_ids.push(cid); }
            else { out.push(Thread { id: root, path: c["path"].as_str().map(String::from), line: c["line"].as_u64().map(|l| l as u32), resolved: false, comments: vec![(who, body)], comment_ids: vec![cid], node_id: None }); }
        }
        // Resolved state and the ids needed to resolve live only in GraphQL; failing here keeps the plain threads.
        if let Ok(map) = self.graphql_threads(r).await { for t in out.iter_mut() { if let Some((node, done)) = map.get(&t.id) { t.node_id = Some(node.clone()); t.resolved = *done; } } }
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
        let gql = self.graphql_url();
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
    async fn current_user(&self) -> Result<String> {
        let v: Value = check(self.req(reqwest::Method::GET, "/user").send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v["login"].as_str().unwrap_or("").to_string())
    }
    async fn my_reviews(&self, host: &str, role: Role, state: StateFilter) -> Result<Vec<RemoteMr>> {
        if self.token.is_empty() { return Err(Error::Unauthorized); }
        let r = match role { Role::ReviewRequested => "review-requested:@me", Role::Authored => "author:@me", Role::Assigned => "assignee:@me" };
        let st = match state { StateFilter::Open => "+is:open", StateFilter::Merged => "+is:merged", StateFilter::Closed => "+is:closed+is:unmerged", StateFilter::All => "" };
        let v: Value = check(self.req(reqwest::Method::GET, &format!("/search/issues?q=is:pr+{r}{st}&sort=updated&order=desc&per_page=50")).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v["items"].as_array().cloned().unwrap_or_default().iter().map(|i| {
            let merged = i.pointer("/pull_request/merged_at").map(|m| !m.is_null()).unwrap_or(false);
            let state = if merged { "merged" } else if i["state"] == "open" { "open" } else { "closed" };
            let project = i["repository_url"].as_str().and_then(|u| u.split("/repos/").nth(1)).unwrap_or("").to_string();
            RemoteMr { url: i["html_url"].as_str().unwrap_or("").into(), title: i["title"].as_str().unwrap_or("").into(), project, number: i["number"].as_u64().unwrap_or(0),
                author: i.pointer("/user/login").and_then(|x| x.as_str()).unwrap_or("").into(), state: state.into(), updated: i["updated_at"].as_str().unwrap_or("").into(), host: host.into(), kind: Kind::GitHub }
        }).collect())
    }
}
