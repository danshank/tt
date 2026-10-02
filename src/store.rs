use crate::domain::{SessionLink, TicketLink, Todo, TodoId, Tree};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

pub struct Store {
    conn: Connection,
}

fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("TT_HOME") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share/tt")
}

impl Store {
    pub fn open_default() -> Result<Self> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        Self::open(&dir.join("tt.db"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS todos (
                 id INTEGER PRIMARY KEY,
                 parent_id INTEGER REFERENCES todos(id) ON DELETE CASCADE,
                 position INTEGER NOT NULL,
                 title TEXT NOT NULL,
                 done_at TEXT,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS session_links (
                 todo_id INTEGER NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
                 session_id TEXT NOT NULL,
                 cwd TEXT NOT NULL,
                 linked_at TEXT NOT NULL,
                 PRIMARY KEY (todo_id, session_id)
             );
             CREATE TABLE IF NOT EXISTS ticket_links (
                 todo_id INTEGER NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
                 key TEXT NOT NULL,
                 PRIMARY KEY (todo_id, key)
             );
             CREATE TABLE IF NOT EXISTS reflections (
                 id INTEGER PRIMARY KEY,
                 started_at TEXT NOT NULL,
                 ended_at TEXT NOT NULL,
                 body TEXT NOT NULL
             );",
        )?;
        Ok(Self { conn })
    }

    pub fn load(&self) -> Result<Tree> {
        let mut st = self.conn.prepare("SELECT id, parent_id, position, title, done_at, created_at FROM todos")?;
        let todos = st
            .query_map([], |r| {
                Ok(Todo {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    position: r.get(2)?,
                    title: r.get(3)?,
                    done_at: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut st = self.conn.prepare("SELECT todo_id, session_id, cwd, linked_at FROM session_links")?;
        let sessions = st
            .query_map([], |r| {
                Ok(SessionLink { todo_id: r.get(0)?, session_id: r.get(1)?, cwd: r.get(2)?, linked_at: r.get(3)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut st = self.conn.prepare("SELECT todo_id, key FROM ticket_links")?;
        let tickets = st
            .query_map([], |r| Ok(TicketLink { todo_id: r.get(0)?, key: r.get(1)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Tree { todos, sessions, tickets })
    }

    fn require(&self, id: TodoId) -> Result<()> {
        let found: Option<i64> = self.conn.query_row("SELECT id FROM todos WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        if found.is_none() {
            bail!("no todo #{id}");
        }
        Ok(())
    }

    fn sibling_ids(&self, parent: Option<TodoId>) -> Result<Vec<TodoId>> {
        let mut st = self.conn.prepare("SELECT id FROM todos WHERE parent_id IS ?1 ORDER BY position, id")?;
        let ids = st.query_map([parent], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    fn parent_of(&self, id: TodoId) -> Result<Option<TodoId>> {
        Ok(self.conn.query_row("SELECT parent_id FROM todos WHERE id = ?1", [id], |r| r.get(0))?)
    }

    /// Put `id` under `parent` at `index` among its siblings, renumbering both old and new sibling lists.
    pub fn place(&self, id: TodoId, parent: Option<TodoId>, index: usize) -> Result<()> {
        if let Some(p) = parent {
            if self.load()?.is_within(p, id) {
                bail!("can't move a todo inside itself");
            }
        }
        let old_parent = self.parent_of(id)?;
        let mut sibs: Vec<TodoId> = self.sibling_ids(parent)?.into_iter().filter(|s| *s != id).collect();
        sibs.insert(index.min(sibs.len()), id);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE todos SET parent_id = ?1 WHERE id = ?2", params![parent, id])?;
        for (i, s) in sibs.iter().enumerate() {
            tx.execute("UPDATE todos SET position = ?1 WHERE id = ?2", params![i as i64, s])?;
        }
        tx.commit()?;
        if old_parent != parent {
            self.renumber(old_parent)?;
        }
        Ok(())
    }

    fn renumber(&self, parent: Option<TodoId>) -> Result<()> {
        for (i, s) in self.sibling_ids(parent)?.iter().enumerate() {
            self.conn.execute("UPDATE todos SET position = ?1 WHERE id = ?2", params![i as i64, s])?;
        }
        Ok(())
    }

    pub fn add(&self, title: &str, parent: Option<TodoId>, index: Option<usize>) -> Result<TodoId> {
        let title = title.trim();
        if title.is_empty() {
            bail!("empty title");
        }
        if let Some(p) = parent {
            self.require(p)?;
        }
        self.conn.execute(
            "INSERT INTO todos (parent_id, position, title, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![parent, i64::MAX, title, now()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.place(id, parent, index.unwrap_or(usize::MAX))?;
        Ok(id)
    }

    pub fn rename(&self, id: TodoId, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            bail!("empty title");
        }
        self.require(id)?;
        self.conn.execute("UPDATE todos SET title = ?1 WHERE id = ?2", params![title, id])?;
        Ok(())
    }

    pub fn set_done(&self, id: TodoId, done: bool) -> Result<()> {
        self.require(id)?;
        let at = done.then(now);
        self.conn.execute("UPDATE todos SET done_at = ?1 WHERE id = ?2", params![at, id])?;
        Ok(())
    }

    pub fn delete(&self, id: TodoId) -> Result<()> {
        let parent = self.parent_of(id)?;
        self.conn.execute("DELETE FROM todos WHERE id = ?1", [id])?;
        self.renumber(parent)
    }

    /// Make `id` the last child of its previous sibling.
    pub fn indent(&self, id: TodoId) -> Result<()> {
        let parent = self.parent_of(id)?;
        let sibs = self.sibling_ids(parent)?;
        let idx = sibs.iter().position(|s| *s == id).unwrap_or(0);
        if idx == 0 {
            bail!("nothing above to nest under");
        }
        self.place(id, Some(sibs[idx - 1]), usize::MAX)
    }

    /// Move `id` out one level, right after its current parent.
    pub fn outdent(&self, id: TodoId) -> Result<()> {
        let Some(parent) = self.parent_of(id)? else { bail!("already top level") };
        let grand = self.parent_of(parent)?;
        let idx = self.sibling_ids(grand)?.iter().position(|s| *s == parent).unwrap_or(0);
        self.place(id, grand, idx + 1)
    }

    pub fn shift(&self, id: TodoId, delta: i64) -> Result<()> {
        let parent = self.parent_of(id)?;
        let sibs = self.sibling_ids(parent)?;
        let idx = sibs.iter().position(|s| *s == id).unwrap_or(0) as i64;
        let to = (idx + delta).clamp(0, sibs.len() as i64 - 1);
        self.place(id, parent, to as usize)
    }

    pub fn link_session(&self, id: TodoId, session_id: &str, cwd: &str) -> Result<()> {
        self.require(id)?;
        self.conn.execute(
            "INSERT INTO session_links (todo_id, session_id, cwd, linked_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (todo_id, session_id) DO UPDATE SET cwd = excluded.cwd, linked_at = excluded.linked_at",
            params![id, session_id, cwd, now()],
        )?;
        Ok(())
    }

    pub fn unlink_session(&self, id: TodoId, session_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM session_links WHERE todo_id = ?1 AND session_id = ?2", params![id, session_id])?;
        Ok(())
    }

    pub fn link_ticket(&self, id: TodoId, key: &str) -> Result<()> {
        self.require(id)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO ticket_links (todo_id, key) VALUES (?1, ?2)",
            params![id, key.trim().to_uppercase()],
        )?;
        Ok(())
    }

    pub fn add_reflection(&self, started_at: &str, ended_at: &str, body: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reflections (started_at, ended_at, body) VALUES (?1, ?2, ?3)",
            params![started_at, ended_at, body],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn titles(s: &Store) -> Vec<(usize, String)> {
        let t = s.load().unwrap();
        t.rows(&HashSet::new(), &|_| false).iter().map(|r| (r.depth, t.get(r.id).unwrap().title.clone())).collect()
    }

    #[test]
    fn add_nest_and_reorder() {
        let s = Store::in_memory().unwrap();
        let a = s.add("a", None, None).unwrap();
        let b = s.add("b", None, None).unwrap();
        let c = s.add("c", None, None).unwrap();
        s.indent(b).unwrap();
        s.indent(c).unwrap();
        assert_eq!(titles(&s), vec![(0, "a".into()), (1, "b".into()), (1, "c".into())]);
        s.shift(c, -1).unwrap();
        assert_eq!(titles(&s), vec![(0, "a".into()), (1, "c".into()), (1, "b".into())]);
        s.outdent(c).unwrap();
        assert_eq!(titles(&s), vec![(0, "a".into()), (1, "b".into()), (0, "c".into())]);
        s.add("a2", None, Some(1)).unwrap();
        assert_eq!(titles(&s)[2], (0, "a2".into()));
        let _ = a;
    }

    #[test]
    fn rejects_cycles() {
        let s = Store::in_memory().unwrap();
        let a = s.add("a", None, None).unwrap();
        let b = s.add("b", Some(a), None).unwrap();
        assert!(s.place(a, Some(b), 0).is_err());
        assert!(s.place(a, Some(a), 0).is_err());
    }

    #[test]
    fn delete_cascades_links() {
        let s = Store::in_memory().unwrap();
        let a = s.add("a", None, None).unwrap();
        let b = s.add("b", Some(a), None).unwrap();
        s.link_session(b, "sid", "/tmp").unwrap();
        s.link_ticket(b, "pe-1").unwrap();
        s.delete(a).unwrap();
        let t = s.load().unwrap();
        assert!(t.todos.is_empty() && t.sessions.is_empty() && t.tickets.is_empty());
    }

    #[test]
    fn hide_done_hides_subtree() {
        let s = Store::in_memory().unwrap();
        let a = s.add("a", None, None).unwrap();
        s.add("b", Some(a), None).unwrap();
        s.set_done(a, true).unwrap();
        assert!(s.load().unwrap().rows(&HashSet::new(), &|t| t.is_done()).is_empty());
    }
}
