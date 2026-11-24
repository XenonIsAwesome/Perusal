#![allow(clippy::type_complexity)]
//! SQLite persistence: reviews, viewed marks, drafts, journal. Tokens never go here.
use rusqlite::{params, Connection, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Draft { pub id: i64, pub file: Option<String>, pub start_line: Option<u32>, pub end_line: Option<u32>, pub body: String, pub ai: bool }

#[derive(Debug, Clone, PartialEq)]
pub struct RecentRow { pub id: i64, pub key: String, pub title: String, pub viewed: u32, pub total: u32, pub drafts: u32, pub pinned: bool, pub state: String }

pub struct Store { conn: Connection }

impl Store {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        // WAL + synchronous NORMAL: every change is durable across app crashes.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS reviews(id INTEGER PRIMARY KEY, key TEXT UNIQUE, title TEXT, head TEXT, pinned INTEGER DEFAULT 0,
                state TEXT DEFAULT 'open', total_files INTEGER DEFAULT 0, last_opened INTEGER DEFAULT 0);
            CREATE TABLE IF NOT EXISTS viewed(review INTEGER, path TEXT, PRIMARY KEY(review, path));
            CREATE TABLE IF NOT EXISTS drafts(id INTEGER PRIMARY KEY, review INTEGER, file TEXT, start_line INTEGER, end_line INTEGER, body TEXT, ai INTEGER DEFAULT 0);
            CREATE TABLE IF NOT EXISTS chat(id INTEGER PRIMARY KEY, review INTEGER, role TEXT, text TEXT);
            CREATE TABLE IF NOT EXISTS journal(id INTEGER PRIMARY KEY, review INTEGER, at INTEGER, event TEXT);")?;
        Ok(Store { conn })
    }
    pub fn memory() -> Result<Self> { Self::open(":memory:") }

    fn log(&self, review: i64, event: &str) -> Result<()> {
        self.conn.execute("INSERT INTO journal(review, at, event) VALUES(?1, strftime('%s','now'), ?2)", params![review, event])?; Ok(())
    }
    /// Opens (or creates) a review pinned to `head`.
    pub fn open_review(&self, key: &str, title: &str, head: &str, total_files: u32) -> Result<i64> {
        self.conn.execute("INSERT INTO reviews(key,title,head,total_files) VALUES(?1,?2,?3,?4) ON CONFLICT(key) DO UPDATE SET last_opened=strftime('%s','now'), total_files=?4",
            params![key, title, head, total_files])?;
        self.conn.query_row("SELECT id FROM reviews WHERE key=?1", params![key], |r| r.get(0))
    }
    pub fn pinned_head(&self, review: i64) -> Result<String> { self.conn.query_row("SELECT head FROM reviews WHERE id=?1", params![review], |r| r.get(0)) }
    pub fn set_viewed(&self, review: i64, path: &str, viewed: bool) -> Result<()> {
        if viewed { self.conn.execute("INSERT OR IGNORE INTO viewed VALUES(?1,?2)", params![review, path])?; }
        else { self.conn.execute("DELETE FROM viewed WHERE review=?1 AND path=?2", params![review, path])?; }
        self.log(review, &format!("viewed {path} {viewed}"))
    }
    pub fn viewed(&self, review: i64) -> Result<Vec<String>> {
        let mut s = self.conn.prepare("SELECT path FROM viewed WHERE review=?1 ORDER BY path")?;
        let r = s.query_map(params![review], |r| r.get(0))?.collect(); r
    }
    /// (viewed, total) for the progress bar.
    pub fn progress(&self, review: i64) -> Result<(u32, u32)> {
        let v: u32 = self.conn.query_row("SELECT COUNT(*) FROM viewed WHERE review=?1", params![review], |r| r.get(0))?;
        let t: u32 = self.conn.query_row("SELECT total_files FROM reviews WHERE id=?1", params![review], |r| r.get(0))?;
        Ok((v, t))
    }
    pub fn add_draft(&self, review: i64, file: Option<&str>, lines: Option<(u32, u32)>, body: &str, ai: bool) -> Result<i64> {
        self.conn.execute("INSERT INTO drafts(review,file,start_line,end_line,body,ai) VALUES(?1,?2,?3,?4,?5,?6)",
            params![review, file, lines.map(|l| l.0), lines.map(|l| l.1), body, ai])?;
        let id = self.conn.last_insert_rowid(); self.log(review, &format!("draft+ {id}"))?; Ok(id)
    }
    pub fn edit_draft(&self, id: i64, body: &str) -> Result<()> { self.conn.execute("UPDATE drafts SET body=?2 WHERE id=?1", params![id, body])?; Ok(()) }
    pub fn delete_draft(&self, id: i64) -> Result<()> { self.conn.execute("DELETE FROM drafts WHERE id=?1", params![id])?; Ok(()) }
    pub fn drafts(&self, review: i64) -> Result<Vec<Draft>> {
        let mut s = self.conn.prepare("SELECT id,file,start_line,end_line,body,ai FROM drafts WHERE review=?1 ORDER BY id")?;
        let r = s.query_map(params![review], |r| Ok(Draft { id: r.get(0)?, file: r.get(1)?, start_line: r.get(2)?, end_line: r.get(3)?, body: r.get(4)?, ai: r.get(5)? }))?.collect(); r
    }
    /// Markdown export of the drafts, grouped by file.
    pub fn export_markdown(&self, review: i64) -> Result<String> {
        let title: String = self.conn.query_row("SELECT title FROM reviews WHERE id=?1", params![review], |r| r.get(0))?;
        let mut out = format!("# Review: {title}\n");
        for d in self.drafts(review)? {
            let loc = match (&d.file, d.start_line, d.end_line) {
                (Some(f), Some(a), Some(b)) if a != b => format!("`{f}:{a}-{b}`"), (Some(f), Some(a), _) => format!("`{f}:{a}`"),
                (Some(f), ..) => format!("`{f}`"), _ => "General".into() };
            out += &format!("\n- **{loc}**{} {}\n", if d.ai { " _(AI)_" } else { "" }, d.body.replace('\n', "\n  "));
        }
        Ok(out)
    }
    /// Moves a review to a new head, re-anchoring drafts via `content(file, which)` where which = old/new head.
    /// Returns ids of drafts that could not be re-anchored (flagged for manual placement).
    pub fn move_head(&self, review: i64, new_head: &str, content: &dyn Fn(&str, bool) -> Option<String>) -> Result<Vec<i64>> {
        let mut lost = Vec::new();
        for d in self.drafts(review)? {
            let (Some(f), Some(a), Some(b)) = (&d.file, d.start_line, d.end_line) else { continue };
            let moved = match (content(f, true), content(f, false)) {
                (Some(o), Some(n)) => perusal_diff::text::reanchor(&o, &n, (a, b)), _ => None };
            match moved {
                Some((x, y)) => { self.conn.execute("UPDATE drafts SET start_line=?2,end_line=?3 WHERE id=?1", params![d.id, x, y])?; }
                None => lost.push(d.id),
            }
        }
        self.conn.execute("UPDATE reviews SET head=?2 WHERE id=?1", params![review, new_head])?;
        self.log(review, &format!("moved head {new_head}"))?;
        Ok(lost)
    }
    /// Docked chat is saved with the review.
    pub fn add_message(&self, review: i64, role: &str, text: &str) -> Result<()> { self.conn.execute("INSERT INTO chat(review,role,text) VALUES(?1,?2,?3)", params![review, role, text])?; Ok(()) }
    pub fn messages(&self, review: i64) -> Result<Vec<(String, String)>> {
        let mut s = self.conn.prepare("SELECT role,text FROM chat WHERE review=?1 ORDER BY id")?; let r = s.query_map(params![review], |r| Ok((r.get(0)?, r.get(1)?)))?.collect(); r
    }
    pub fn set_state(&self, review: i64, state: &str) -> Result<()> { self.conn.execute("UPDATE reviews SET state=?2 WHERE id=?1", params![review, state])?; Ok(()) }
    pub fn set_pinned(&self, review: i64, pinned: bool) -> Result<()> { self.conn.execute("UPDATE reviews SET pinned=?2 WHERE id=?1", params![review, pinned])?; Ok(()) }
    /// Retention: reviews stay until submitted or discarded; submitted/merged ones expire after `days`; pinned never expire.
    /// `now` is unix seconds. Returns how many were removed.
    pub fn expire(&self, now: i64, days: i64) -> Result<usize> {
        let mut s = self.conn.prepare("SELECT id FROM reviews WHERE pinned=0 AND state IN ('submitted','merged') AND last_opened < ?1")?;
        let ids: Vec<i64> = s.query_map(params![now - days * 86400], |r| r.get(0))?.collect::<Result<_>>()?;
        for id in &ids { self.discard(*id)?; }
        Ok(ids.len())
    }
    pub fn discard(&self, review: i64) -> Result<()> {
        for t in ["viewed", "drafts", "journal", "chat"] { self.conn.execute(&format!("DELETE FROM {t} WHERE review=?1"), params![review])?; }
        self.conn.execute("DELETE FROM reviews WHERE id=?1", params![review])?; Ok(())
    }
    /// Rows for the recent-reviews page, pinned first then most recently opened.
    pub fn recent_rows(&self) -> Result<Vec<RecentRow>> {
        let mut s = self.conn.prepare("SELECT r.id, r.key, r.title, (SELECT COUNT(*) FROM viewed v WHERE v.review=r.id), r.total_files,
            (SELECT COUNT(*) FROM drafts d WHERE d.review=r.id), r.pinned, r.state FROM reviews r ORDER BY r.pinned DESC, r.last_opened DESC, r.id DESC")?;
        let r = s.query_map([], |r| Ok(RecentRow { id: r.get(0)?, key: r.get(1)?, title: r.get(2)?, viewed: r.get(3)?, total: r.get(4)?, drafts: r.get(5)?, pinned: r.get::<_, i64>(6)? != 0, state: r.get(7)? }))?.collect(); r
    }
    /// Recent-reviews page rows: (id, title, viewed, total, drafts, pinned).
    pub fn recent(&self) -> Result<Vec<(i64, String, u32, u32, u32, bool)>> {
        let mut s = self.conn.prepare("SELECT r.id, r.title, (SELECT COUNT(*) FROM viewed v WHERE v.review=r.id), r.total_files,
            (SELECT COUNT(*) FROM drafts d WHERE d.review=r.id), r.pinned FROM reviews r ORDER BY r.pinned DESC, r.last_opened DESC, r.id DESC")?;
        let r = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get::<_, i64>(5)? != 0)))?.collect(); r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn review_lifecycle() {
        let s = Store::memory().unwrap();
        let r = s.open_review("gh/o/r#1", "Fix", "abc", 4).unwrap();
        assert_eq!(s.open_review("gh/o/r#1", "Fix", "zzz", 4).unwrap(), r);
        assert_eq!(s.pinned_head(r).unwrap(), "abc"); // pinned to first head
        s.set_viewed(r, "a.rs", true).unwrap(); s.set_viewed(r, "b.rs", true).unwrap(); s.set_viewed(r, "b.rs", false).unwrap();
        assert_eq!(s.progress(r).unwrap(), (1, 4));
        let d = s.add_draft(r, Some("a.rs"), Some((3, 5)), "why?", false).unwrap();
        s.add_draft(r, None, None, "overall ok", true).unwrap();
        s.edit_draft(d, "why this?").unwrap();
        let md = s.export_markdown(r).unwrap();
        assert!(md.contains("`a.rs:3-5`") && md.contains("why this?") && md.contains("(AI)"));
        assert_eq!(s.recent().unwrap()[0].4, 2);
        let row = &s.recent_rows().unwrap()[0]; assert_eq!((row.key.as_str(), row.viewed, row.total, row.drafts), ("gh/o/r#1", 1, 4, 2));
        s.discard(r).unwrap(); assert!(s.recent().unwrap().is_empty());
    }
    #[test] fn move_head_reanchors() {
        let s = Store::memory().unwrap(); let r = s.open_review("k", "t", "h1", 1).unwrap();
        let ok = s.add_draft(r, Some("f"), Some((3, 3)), "keep", false).unwrap();
        let gone = s.add_draft(r, Some("f"), Some((2, 2)), "edited line", false).unwrap();
        let lost = s.move_head(r, "h2", &|_, old| Some(if old { "a\nb\nc\n".into() } else { "z\na\nB\nc\n".into() })).unwrap();
        assert_eq!(lost, vec![gone]); assert_eq!(s.pinned_head(r).unwrap(), "h2");
        assert_eq!(s.drafts(r).unwrap().iter().find(|d| d.id == ok).unwrap().start_line, Some(4));
    }
    #[test] fn chat_saved_and_answer_becomes_draft() {
        let s = Store::memory().unwrap(); let r = s.open_review("k", "t", "h", 1).unwrap();
        s.add_message(r, "user", "why?").unwrap(); s.add_message(r, "agent", "because").unwrap();
        assert_eq!(s.messages(r).unwrap().len(), 2);
        let m = s.messages(r).unwrap(); s.add_draft(r, Some("a"), Some((1, 1)), &m[1].1, true).unwrap(); assert!(s.drafts(r).unwrap()[0].ai);
        s.discard(r).unwrap(); assert!(s.messages(r).unwrap().is_empty());
    }
    #[test] fn retention() {
        let s = Store::memory().unwrap();
        let a = s.open_review("a", "a", "h", 1).unwrap(); let b = s.open_review("b", "b", "h", 1).unwrap(); let c = s.open_review("c", "c", "h", 1).unwrap(); let d = s.open_review("d", "d", "h", 1).unwrap();
        s.set_state(a, "submitted").unwrap(); s.set_state(b, "submitted").unwrap(); s.set_pinned(b, true).unwrap(); s.set_state(c, "merged").unwrap();
        let _ = d; // open and unsubmitted: kept forever
        let far = 4_000_000_000; assert_eq!(s.expire(far, 30).unwrap(), 2);
        let left: Vec<String> = s.recent().unwrap().into_iter().map(|r| r.1).collect(); assert!(left.contains(&"b".to_string()) && left.contains(&"d".to_string()) && left.len() == 2);
        assert_eq!(s.expire(0, 30).unwrap(), 0);
    }
    #[test] fn survives_reopen_on_disk() {
        let p = std::env::temp_dir().join(format!("perusal-db-{}.sqlite", std::process::id())); let ps = p.to_str().unwrap();
        { let s = Store::open(ps).unwrap(); let r = s.open_review("k", "t", "h", 1).unwrap(); s.add_draft(r, None, None, "kept", false).unwrap(); }
        let s = Store::open(ps).unwrap(); let r = s.open_review("k", "t", "h", 1).unwrap();
        assert_eq!(s.drafts(r).unwrap()[0].body, "kept");
        for e in ["", "-wal", "-shm"] { let _ = std::fs::remove_file(format!("{ps}{e}")); }
    }
}
