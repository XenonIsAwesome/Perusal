//! SQLite persistence: reviews, viewed marks, drafts, journal. Tokens never go here.
use rusqlite::{params, Connection, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Draft { pub id: i64, pub file: Option<String>, pub start_line: Option<u32>, pub end_line: Option<u32>, pub body: String, pub ai: bool }

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
    pub fn discard(&self, review: i64) -> Result<()> {
        for t in ["viewed", "drafts", "journal"] { self.conn.execute(&format!("DELETE FROM {t} WHERE review=?1"), params![review])?; }
        self.conn.execute("DELETE FROM reviews WHERE id=?1", params![review])?; Ok(())
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
        s.discard(r).unwrap(); assert!(s.recent().unwrap().is_empty());
    }
    #[test] fn survives_reopen_on_disk() {
        let p = std::env::temp_dir().join(format!("perusal-db-{}.sqlite", std::process::id())); let ps = p.to_str().unwrap();
        { let s = Store::open(ps).unwrap(); let r = s.open_review("k", "t", "h", 1).unwrap(); s.add_draft(r, None, None, "kept", false).unwrap(); }
        let s = Store::open(ps).unwrap(); let r = s.open_review("k", "t", "h", 1).unwrap();
        assert_eq!(s.drafts(r).unwrap()[0].body, "kept");
        for e in ["", "-wal", "-shm"] { let _ = std::fs::remove_file(format!("{ps}{e}")); }
    }
}
