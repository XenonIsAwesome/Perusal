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
    /// Every pull request in the repositories you can push to (the ones you can review with effect), newest activity first.
    /// Capped at the 40 most recently pushed repositories with 30 pull requests each; repositories are queried concurrently
    /// and one failing repository is skipped.
    async fn reviewable(&self, host: &str, state: StateFilter) -> Result<Vec<RemoteMr>> {
        let repos: Vec<Value> = check(self.req(reqwest::Method::GET, "/user/repos?affiliation=owner,collaborator,organization_member&sort=pushed&per_page=100").send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let names: Vec<String> = repos.iter().filter(|r| r["archived"].as_bool() != Some(true) && r.pointer("/permissions/push").and_then(|x| x.as_bool()).unwrap_or(false)).filter_map(|r| r["full_name"].as_str().map(String::from)).take(40).collect();
        let st = match state { StateFilter::Open => "open", StateFilter::All => "all", _ => "closed" };
        // Queried in small waves: firing 40 requests at once trips GitHub's abuse limits, and the repositories that then fail would silently be missing.
        let mut results = vec![];
        for wave in names.chunks(8) {
            let mut set = tokio::task::JoinSet::new();
            for n in wave.iter().cloned() {
                let (http, api, token) = (self.http.clone(), self.api.clone(), self.token.clone());
                set.spawn(async move {
                    let g = GitHub { http, api, token };
                    let url = format!("/repos/{n}/pulls?state={st}&sort=updated&direction=desc&per_page=30");
                    let mut last = None;
                    for attempt in 0..2 {
                        if attempt > 0 { tokio::time::sleep(std::time::Duration::from_millis(400)).await; }
                        match check(g.req(reqwest::Method::GET, &url).send().await).await {
                            Ok(r) => return match r.json::<Vec<Value>>().await { Ok(v) => Ok((n, v)), Err(e) => Err(Error::Offline(e.to_string())) },
                            Err(Error::Unauthorized) => return Err(Error::Unauthorized),
                            Err(e) => last = Some(e),
                        }
                    }
                    Err(last.unwrap_or(Error::Offline("no answer".into())))
                });
            }
            while let Some(r) = set.join_next().await { if let Ok(r) = r { results.push(r); } }
        }
        let (mut out, mut requested, mut first_err, mut ok_any) = (vec![], vec![], None, false);
        // Pull requests that ask for your review in repositories outside the capped list (or where you cannot push) come from search.
        if matches!(state, StateFilter::Open | StateFilter::All) {
            let q = format!("/search/issues?q=is:pr+review-requested:@me{}&sort=updated&order=desc&per_page=50", if state == StateFilter::Open { "+is:open" } else { "" });
            if let Ok(resp) = check(self.req(reqwest::Method::GET, &q).send().await).await { if let Ok(v) = resp.json::<Value>().await { requested.extend(v["items"].as_array().into_iter().flatten().map(|i| search_row(i, host))); } }
        }
        for r in results {
            let (project, list) = match r { Ok(v) => v, Err(e) => { first_err.get_or_insert(e); continue } };
            ok_any = true;
            for p in list {
                let merged = !p["merged_at"].is_null();
                let state_s = if merged { "merged" } else if p["state"] == "open" { "open" } else { "closed" };
                if (state == StateFilter::Merged && !merged) || (state == StateFilter::Closed && merged) { continue; }
                out.push(RemoteMr { url: p["html_url"].as_str().unwrap_or("").into(), title: p["title"].as_str().unwrap_or("").into(), project: project.clone(), number: p["number"].as_u64().unwrap_or(0),
                    author: p.pointer("/user/login").and_then(|x| x.as_str()).unwrap_or("").into(), state: state_s.into(), updated: p["updated_at"].as_str().unwrap_or("").into(), host: host.into(), kind: Kind::GitHub,
                    author_avatar: p.pointer("/user/avatar_url").and_then(|x| x.as_str()).unwrap_or("").into(), assignees: people(&p["assignees"], "login"), reviewers: people(&p["requested_reviewers"], "login"),
                    labels: p["labels"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str().map(String::from)).collect(), description: p["body"].as_str().unwrap_or("").into(), draft: p["draft"].as_bool() == Some(true) });
            }
        }
        // the same pull request can come from both sources: keep the repository listing's richer row (it has the reviewer list)
        let mut seen = std::collections::HashSet::new();
        out.extend(requested);
        out.retain(|m| seen.insert(m.url.clone()));
        // every repository failing (rate limit, expired token) is an error, not an empty list
        match first_err { Some(e) if !ok_any && out.is_empty() => Err(e), _ => Ok(out) }
    }
    fn graphql_url(&self) -> String { if self.api.ends_with("/api/v3") { self.api.replace("/api/v3", "/api/graphql") } else { format!("{}/graphql", self.api) } }
    /// Review threads by the database id of their first comment: (GraphQL node id, resolved). Paged (capped at 50 pages); empty on any failure.
    async fn graphql_threads(&self, r: &MrRef) -> std::collections::HashMap<String, (Option<String>, bool)> {
        let mut m = std::collections::HashMap::new();
        let Some((owner, name)) = r.project.split_once('/') else { return m };
        let gql = self.graphql_url(); let mut cursor = Value::Null;
        for _ in 0..50 {
            let q = json!({"query": "query($o:String!,$n:String!,$p:Int!,$c:String){repository(owner:$o,name:$n){pullRequest(number:$p){reviewThreads(first:100,after:$c){pageInfo{hasNextPage endCursor} nodes{id isResolved comments(first:1){nodes{databaseId}}}}}}}",
                "variables": {"o": owner, "n": name, "p": r.number, "c": cursor}});
            let Ok(resp) = check(self.http.post(&gql).bearer_auth(&self.token).json(&q).send().await).await else { break };
            let Ok(v) = resp.json::<Value>().await else { break };
            let Some(rt) = v.pointer("/data/repository/pullRequest/reviewThreads") else { break };
            for n in rt["nodes"].as_array().into_iter().flatten() {
                if let Some(db) = n.pointer("/comments/nodes/0/databaseId").and_then(|x| x.as_u64()) { m.insert(db.to_string(), (n["id"].as_str().map(String::from), n["isResolved"].as_bool() == Some(true))); }
            }
            if rt.pointer("/pageInfo/hasNextPage").and_then(|x| x.as_bool()) != Some(true) { break; }
            cursor = rt.pointer("/pageInfo/endCursor").cloned().unwrap_or(Value::Null);
            if cursor.is_null() { break; }
        }
        m
    }
}

#[async_trait::async_trait]
impl Provider for GitHub {
    async fn mr_info(&self, r: &MrRef) -> Result<MrInfo> {
        let v: Value = check(self.req(reqwest::Method::GET, &format!("/repos/{}/pulls/{}", r.project, r.number)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let s = |p: &str| v.pointer(p).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let state = if v["merged"].as_bool() == Some(true) { "merged".into() } else { s("/state") };
        Ok(MrInfo { title: s("/title"), state, author: s("/user/login"), head_sha: s("/head/sha"), base_ref: s("/base/ref"), web_url: s("/html_url"), description: s("/body"), source_ref: s("/head/ref"),
            author_avatar: s("/user/avatar_url"), assignees: people(&v["assignees"], "login"), reviewers: people(&v["requested_reviewers"], "login") })
    }
    async fn threads(&self, r: &MrRef) -> Result<Vec<Thread>> {
        let v: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/repos/{}/pulls/{}/comments", r.project, r.number)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let mut out: Vec<Thread> = Vec::new();
        for c in v {
            let root = c["in_reply_to_id"].as_u64().map(|x| x.to_string()).unwrap_or(c["id"].to_string());
            let who = c["user"]["login"].as_str().unwrap_or("").to_string(); let body = c["body"].as_str().unwrap_or("").to_string(); let av = c["user"]["avatar_url"].as_str().unwrap_or("").to_string();
            let cid = c["id"].to_string();
            if let Some(t) = out.iter_mut().find(|t| t.id == root) { t.comments.push((who, body)); t.avatars.push(av); t.comment_ids.push(cid); }
            else { out.push(Thread { id: root, path: c["path"].as_str().map(String::from), line: c["line"].as_u64().map(|l| l as u32), resolved: false, comments: vec![(who, body)], avatars: vec![av], comment_ids: vec![cid], node_id: None }); }
        }
        // the REST API does not say which review threads are resolved nor gives their GraphQL ids; ask GraphQL (best effort)
        for (root, (node, done)) in self.graphql_threads(r).await { if let Some(t) = out.iter_mut().find(|t| t.id == root) { t.node_id = node; t.resolved = done; } }
        // general conversation on the pull request (issue comments); optional, so a failure here keeps the inline threads
        if let Ok(resp) = check(self.req(reqwest::Method::GET, &format!("/repos/{}/issues/{}/comments?per_page=100", r.project, r.number)).send().await).await {
            if let Ok(list) = resp.json::<Vec<Value>>().await {
                for c in list { out.push(Thread { id: format!("i{}", c["id"]), path: None, line: None, resolved: false, comments: vec![(c["user"]["login"].as_str().unwrap_or("").to_string(), c["body"].as_str().unwrap_or("").to_string())], avatars: vec![c["user"]["avatar_url"].as_str().unwrap_or("").to_string()], comment_ids: vec![], node_id: None }); }
            }
        }
        Ok(out)
    }
    /// One review call carrying all comments, so a failure leaves nothing posted.
    async fn submit(&self, r: &MrRef, s: &Submission) -> Result<()> {
        let event = match s.verdict { Verdict::Approve => "APPROVE", Verdict::RequestChanges => "REQUEST_CHANGES", Verdict::Comment => "COMMENT" };
        let comments: Vec<Value> = s.comments.iter().map(|c| if c.start_line != c.end_line {
            json!({"path": c.path, "start_line": c.start_line, "line": c.end_line, "side": if c.old_side { "LEFT" } else { "RIGHT" }, "start_side": if c.old_side { "LEFT" } else { "RIGHT" }, "body": c.body})
        } else { json!({"path": c.path, "line": c.end_line, "side": if c.old_side { "LEFT" } else { "RIGHT" }, "body": c.body}) }).collect();
        let body = json!({"commit_id": s.head_sha, "body": s.summary, "event": event, "comments": comments});
        // A review with nothing to say (no summary, no inline comments, a plain Comment verdict) is rejected by GitHub; skip it when only general comments remain.
        if !(s.verdict == Verdict::Comment && s.summary.is_empty() && s.comments.is_empty() && !s.general.is_empty()) {
            check(self.req(reqwest::Method::POST, &format!("/repos/{}/pulls/{}/reviews", r.project, r.number)).json(&body).send().await).await?;
        }
        // General comments are conversation threads (issue comments), posted after the review that carries the verdict.
        for g in &s.general {
            check(self.req(reqwest::Method::POST, &format!("/repos/{}/issues/{}/comments", r.project, r.number)).json(&json!({"body": g})).send().await).await?;
        }
        Ok(())
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
    async fn my_reactions(&self, r: &MrRef, comments: &[String], emoji: &str) -> Result<Vec<String>> {
        let me = self.current_user().await?;
        let mut set = tokio::task::JoinSet::new();
        for c in comments.iter().take(100) {
            let (g, project, c, emoji, me) = (GitHub { http: self.http.clone(), api: self.api.clone(), token: self.token.clone() }, r.project.clone(), c.clone(), emoji.to_string(), me.clone());
            set.spawn(async move {
                let list: Vec<Value> = check(g.req(reqwest::Method::GET, &format!("/repos/{project}/pulls/comments/{c}/reactions?per_page=100")).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
                Ok::<_, Error>(list.iter().any(|x| x["content"].as_str() == Some(emoji.as_str()) && x.pointer("/user/login").and_then(|l| l.as_str()) == Some(me.as_str())).then_some(c))
            });
        }
        let mut out = vec![];
        while let Some(res) = set.join_next().await { if let Ok(Ok(Some(c))) = res { out.push(c); } }
        Ok(out)
    }
    async fn unreact(&self, r: &MrRef, _thread: &str, comment: &str, emoji: &str) -> Result<()> {
        let me = self.current_user().await?;
        let list: Vec<Value> = check(self.req(reqwest::Method::GET, &format!("/repos/{}/pulls/comments/{comment}/reactions?per_page=100", r.project)).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        let mine = list.iter().find(|x| x["content"].as_str() == Some(emoji) && x.pointer("/user/login").and_then(|l| l.as_str()) == Some(me.as_str())).and_then(|x| x["id"].as_u64());
        let Some(id) = mine else { return Err(Error::Http(404, "you have not reacted with that".into())) };
        check(self.req(reqwest::Method::DELETE, &format!("/repos/{}/pulls/comments/{comment}/reactions/{id}", r.project)).send().await).await.map(|_| ())
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
        if role == Role::Reviewable { return self.reviewable(host, state).await; }
        let r = match role { Role::ReviewRequested => "review-requested:@me", Role::Authored => "author:@me", Role::Assigned => "assignee:@me", Role::Reviewable => "" };
        let st = match state { StateFilter::Open => "+is:open", StateFilter::Merged => "+is:merged", StateFilter::Closed => "+is:closed+is:unmerged", StateFilter::All => "" };
        let v: Value = check(self.req(reqwest::Method::GET, &format!("/search/issues?q=is:pr+{r}{st}&sort=updated&order=desc&per_page=50")).send().await).await?.json().await.map_err(|e| Error::Offline(e.to_string()))?;
        Ok(v["items"].as_array().cloned().unwrap_or_default().iter().map(|i| search_row(i, host)).collect())
    }
}

/// One result of `/search/issues` (a pull request) as a listed row.
fn search_row(i: &Value, host: &str) -> RemoteMr {
    let merged = i.pointer("/pull_request/merged_at").map(|m| !m.is_null()).unwrap_or(false);
    let state = if merged { "merged" } else if i["state"] == "open" { "open" } else { "closed" };
    let project = i["repository_url"].as_str().and_then(|u| u.split("/repos/").nth(1)).unwrap_or("").to_string();
    RemoteMr { url: i["html_url"].as_str().unwrap_or("").into(), title: i["title"].as_str().unwrap_or("").into(), project, number: i["number"].as_u64().unwrap_or(0),
        author: i.pointer("/user/login").and_then(|x| x.as_str()).unwrap_or("").into(), state: state.into(), updated: i["updated_at"].as_str().unwrap_or("").into(), host: host.into(), kind: Kind::GitHub,
        author_avatar: i.pointer("/user/avatar_url").and_then(|x| x.as_str()).unwrap_or("").into(), assignees: people(&i["assignees"], "login"), reviewers: vec![],
        labels: i["labels"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str().map(String::from)).collect(), description: i["body"].as_str().unwrap_or("").into(), draft: i["draft"].as_bool() == Some(true) }
}
