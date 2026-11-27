use crate::*;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};

pub struct GitLab { pub http: reqwest::Client, pub api: String, pub token: String }

impl GitLab {
    /// Every merge request in the projects you are at least a Developer in (the ones you can review with effect), newest activity first.
    /// Capped at the 40 most recently active projects with 30 merge requests each; projects are queried concurrently and one failing project is skipped.
    async fn reviewable(&self, host: &str, state: StateFilter) -> Result<Vec<RemoteMr>> {
        let projects: Vec<Value> = check(self.req(reqwest::Method::GET, "/projects?membership=true&min_access_level=30&archived=false&simple=true&order_by=last_activity_at&sort=desc&per_page=40").send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let st = match state { StateFilter::Open => "opened", StateFilter::Merged => "merged", StateFilter::Closed => "closed", StateFilter::All => "all" };
        let mut set = tokio::task::JoinSet::new();
        for id in projects.iter().filter_map(|p| p["id"].as_u64()) {
            let (http, api, token) = (self.http.clone(), self.api.clone(), self.token.clone());
            set.spawn(async move {
                let g = GitLab { http, api, token };
                let v: Vec<Value> = check(g.req(reqwest::Method::GET, &format!("/projects/{id}/merge_requests?state={st}&order_by=updated_at&sort=desc&per_page=30")).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
                Ok::<_, Error>(v)
            });
        }
        let (mut out, mut first_err, mut ok_any) = (vec![], None, false);
        while let Some(r) = set.join_next().await {
            match r { Ok(Ok(list)) => { ok_any = true; out.extend(list.iter().map(|m| mr_row(m, host))); } Ok(Err(e)) => { first_err.get_or_insert(e); } Err(_) => {} }
        }
        // Requests that ask for your review in projects outside the capped list come from the global listing.
        if let Ok(me) = self.current_user().await {
            let q = format!("/merge_requests?reviewer_username={me}&scope=all&state={st}&order_by=updated_at&sort=desc&per_page=50");
            if let Ok(resp) = check(self.req(reqwest::Method::GET, &q).send().await).await { if let Ok(v) = resp.json::<Vec<Value>>().await { out.extend(v.iter().map(|m| mr_row(m, host))); } }
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|m| seen.insert(m.url.clone()));
        // every project failing (rate limit, expired token) is an error, not an empty list
        match first_err { Some(e) if !ok_any && out.is_empty() => Err(e), _ => Ok(out) }
    }
    pub fn api_base(host: &str) -> String { format!("https://{host}/api/v4") }
    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder { let r = self.http.request(m, format!("{}{path}", self.api)); if self.token.is_empty() { r } else { r.header("PRIVATE-TOKEN", &self.token) } }
    fn mr(r: &MrRef) -> String { format!("/projects/{}/merge_requests/{}", utf8_percent_encode(&r.project, NON_ALPHANUMERIC), r.number) }
}

/// Draft-note body for one inline comment. A multi-line selection needs `line_range` (start and end line codes);
/// `new_line` alone anchors the note to its last line only.
pub fn gitlab_position(c: &InlineComment, base_sha: &Option<String>, head: &str, start_sha: &Option<String>) -> Value {
    let mut pos = json!({"position_type": "text", "old_path": c.path, "new_path": c.path, "base_sha": base_sha, "head_sha": head, "start_sha": start_sha});
    // a comment on a removed line is anchored by its old line number only
    pos[if c.old_side { "old_line" } else { "new_line" }] = json!(c.end_line);
    if c.start_line < c.end_line {
        let hash = sha1_smol::Sha1::from(c.path.as_bytes()).digest().to_string();
        let end = |l: u32, o: u32| if c.old_side { json!({"line_code": format!("{hash}_{l}_0"), "type": "old", "old_line": l}) } else { json!({"line_code": format!("{hash}_{o}_{l}"), "type": "new", "new_line": l}) };
        pos["line_range"] = json!({"start": end(c.start_line, c.start_old), "end": end(c.end_line, c.end_old)});
    }
    json!({"note": c.body, "position": pos})
}

#[async_trait::async_trait]
impl Provider for GitLab {
    async fn mr_info(&self, r: &MrRef) -> Result<MrInfo> {
        let v: Value = check(self.req(reqwest::Method::GET, &Self::mr(r)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let s = |p: &str| v.pointer(p).and_then(|x| x.as_str()).unwrap_or("").to_string();
        Ok(MrInfo { title: s("/title"), state: s("/state"), author: s("/author/username"), head_sha: s("/sha"), base_ref: s("/target_branch"), web_url: s("/web_url"), description: s("/description"), source_ref: s("/source_branch"),
            author_avatar: s("/author/avatar_url"), assignees: people(&v["assignees"], "username"), reviewers: people(&v["reviewers"], "username") })
    }
    async fn threads(&self, r: &MrRef) -> Result<Vec<Thread>> {
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("{}/discussions", Self::mr(r))).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v.iter().filter_map(|d| {
            // system notes ("added 1 commit", "approved this merge request") are not conversation
            let notes: Vec<Value> = d["notes"].as_array().cloned().unwrap_or_default().into_iter().filter(|n| n["system"].as_bool() != Some(true)).collect();
            if notes.is_empty() { return None; }
            let first = notes.first().cloned().unwrap_or(Value::Null);
            Some(Thread { id: d["id"].as_str().unwrap_or("").into(), path: first.pointer("/position/new_path").and_then(|x| x.as_str()).map(String::from),
                line: first.pointer("/position/new_line").and_then(|x| x.as_u64()).map(|l| l as u32), resolved: thread_resolved(&notes),
                comments: notes.iter().map(|n| (n["author"]["username"].as_str().unwrap_or("").into(), n["body"].as_str().unwrap_or("").into())).collect(),
                avatars: notes.iter().map(|n| n["author"]["avatar_url"].as_str().unwrap_or("").into()).collect(),
                comment_ids: notes.iter().map(|n| n["id"].to_string()).collect(), node_id: None })
        }).collect())
    }
    /// Draft notes then bulk publish. If any step fails, drafts already created are deleted: nothing half-posted.
    async fn submit(&self, r: &MrRef, s: &Submission) -> Result<()> {
        if s.verdict == Verdict::RequestChanges { return Err(Error::Unsupported("request changes")); }
        let base = Self::mr(r);
        let mut created: Vec<u64> = Vec::new();
        // Inline positions need the MR's diff_refs; without real SHAs GitLab files the note as a plain MR thread.
        let mr: Value = check(self.req(reqwest::Method::GET, &base).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let refs = |k: &str| mr.pointer(&format!("/diff_refs/{k}")).and_then(|x| x.as_str()).map(String::from);
        let head = refs("head_sha").unwrap_or_else(|| s.head_sha.clone());
        let (base_sha, start_sha) = (refs("base_sha"), refs("start_sha"));
        let mut bodies: Vec<Value> = s.comments.iter().map(|c| gitlab_position(c, &base_sha, &head, &start_sha)).collect();
        if !s.summary.is_empty() { bodies.push(json!({"note": s.summary})); }
        // a draft note without a position publishes as its own discussion thread
        for g in &s.general { bodies.push(json!({"note": g})); }
        let res: Result<()> = async {
            for b in &bodies {
                let v: Value = check(self.req(reqwest::Method::POST, &format!("{base}/draft_notes")).json(b).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
                if let Some(id) = v["id"].as_u64() { created.push(id); }
            }
            check(self.req(reqwest::Method::POST, &format!("{base}/draft_notes/bulk_publish")).send().await).await?;
            if s.verdict == Verdict::Approve {
                // The comments are published by now. A refused approval is answered with 401 even though the token worked above, so it
                // is kept as an HTTP error with the host's own body and marked as the approval step (see `explain_submit_error`).
                let resp = self.req(reqwest::Method::POST, &format!("{base}/approve")).send().await;
                match resp {
                    Ok(r) if r.status().as_u16() == 401 => return Err(Error::Http(401, format!("{APPROVAL_STEP}{}", r.text().await.unwrap_or_default().chars().take(300).collect::<String>()))),
                    other => check(other).await.map(|_| ()).map_err(|e| match e { Error::Http(c, b) => Error::Http(c, format!("{APPROVAL_STEP}{b}")), o => o })?,
                }
            }
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
    async fn my_reactions(&self, r: &MrRef, comments: &[String], emoji: &str) -> Result<Vec<String>> {
        let me = self.current_user().await?;
        let mut set = tokio::task::JoinSet::new();
        for c in comments.iter().take(100) {
            let (g, url, c, emoji, me) = (GitLab { http: self.http.clone(), api: self.api.clone(), token: self.token.clone() }, format!("{}/notes/{c}/award_emoji?per_page=100", Self::mr(r)), c.clone(), emoji.to_string(), me.clone());
            set.spawn(async move {
                let list: Vec<Value> = check(g.req(reqwest::Method::GET, &url).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
                Ok::<_, Error>(list.iter().any(|x| x["name"].as_str() == Some(emoji.as_str()) && x.pointer("/user/username").and_then(|l| l.as_str()) == Some(me.as_str())).then_some(c))
            });
        }
        let mut out = vec![];
        while let Some(res) = set.join_next().await { if let Ok(Ok(Some(c))) = res { out.push(c); } }
        Ok(out)
    }
    async fn unreact(&self, r: &MrRef, _thread: &str, comment: &str, emoji: &str) -> Result<()> {
        let me = self.current_user().await?;
        let list: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("{}/notes/{comment}/award_emoji?per_page=100", Self::mr(r))).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let mine = list.iter().find(|x| x["name"].as_str() == Some(emoji) && x.pointer("/user/username").and_then(|l| l.as_str()) == Some(me.as_str())).and_then(|x| x["id"].as_u64());
        let Some(id) = mine else { return Err(Error::Http(404, "you have not reacted with that".into())) };
        check(self.req(reqwest::Method::DELETE, &format!("{}/notes/{comment}/award_emoji/{id}", Self::mr(r))).send().await).await.map(|_| ())
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
    async fn current_user(&self) -> Result<String> {
        let v: Value = check(self.req(reqwest::Method::GET, "/user").send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v["username"].as_str().unwrap_or("").to_string())
    }
    async fn my_reviews(&self, host: &str, role: Role, state: StateFilter) -> Result<Vec<RemoteMr>> {
        if self.token.is_empty() { return Err(Error::Unauthorized); }
        if role == Role::Reviewable { return self.reviewable(host, state).await; }
        let me = self.current_user().await?;
        let who = match role { Role::ReviewRequested => format!("reviewer_username={me}"), Role::Authored => "scope=created_by_me".into(), Role::Assigned => "scope=assigned_to_me".into(), Role::Reviewable => String::new() };
        let scope = if matches!(role, Role::ReviewRequested) { "&scope=all" } else { "" };
        let st = match state { StateFilter::Open => "opened", StateFilter::Merged => "merged", StateFilter::Closed => "closed", StateFilter::All => "all" };
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/merge_requests?{who}{scope}&state={st}&order_by=updated_at&sort=desc&per_page=50")).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v.iter().map(|m| mr_row(m, host)).collect())
    }
}

/// One listed merge request as a row. Some instances report http:// web_urls (TLS terminated by a proxy); always use the https host the user connected to.
fn mr_row(m: &Value, host: &str) -> RemoteMr {
    let web = m["web_url"].as_str().unwrap_or("");
    let project = web.split_once("://").map(|x| x.1).and_then(|r| r.split_once('/')).map(|x| x.1).and_then(|p| p.split("/-/").next()).unwrap_or("").to_string();
    let state = match m["state"].as_str() { Some("opened") => "open", Some("merged") => "merged", _ => "closed" };
    let number = m["iid"].as_u64().unwrap_or(0);
    let url = if project.is_empty() { web.to_string() } else { format!("https://{host}/{project}/-/merge_requests/{number}") };
    RemoteMr { url, title: m["title"].as_str().unwrap_or("").into(), project, number, author: m.pointer("/author/username").and_then(|x| x.as_str()).unwrap_or("").into(),
        state: state.into(), updated: m["updated_at"].as_str().unwrap_or("").into(), host: host.into(), kind: Kind::GitLab,
        author_avatar: m.pointer("/author/avatar_url").and_then(|x| x.as_str()).unwrap_or("").into(), assignees: people(&m["assignees"], "username"), reviewers: people(&m["reviewers"], "username"),
        labels: m["labels"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect(), description: m["description"].as_str().unwrap_or("").into(), draft: m["draft"].as_bool() == Some(true) }
}

/// A discussion is resolved when it has resolvable notes and all of them are resolved; a note without a `resolvable` field counts as resolvable.
fn thread_resolved(notes: &[Value]) -> bool {
    let mut resolvable = notes.iter().filter(|n| n["resolvable"].as_bool() != Some(false)).peekable();
    resolvable.peek().is_some() && resolvable.all(|n| n["resolved"].as_bool() == Some(true))
}
