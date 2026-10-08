use serde::Serialize;
use std::collections::HashSet;

pub type TodoId = i64;

#[derive(Debug, Clone, Serialize)]
pub struct Todo {
    pub id: TodoId,
    pub parent_id: Option<TodoId>,
    pub position: i64,
    pub title: String,
    pub dir: Option<String>,
    pub done_at: Option<String>,
    pub created_at: String,
}

impl Todo {
    pub fn is_done(&self) -> bool {
        self.done_at.is_some()
    }
}

/// A Claude Code session that has claimed a todo.
#[derive(Debug, Clone, Serialize)]
pub struct SessionLink {
    pub todo_id: TodoId,
    pub session_id: String,
    pub cwd: String,
    pub linked_at: String,
}

/// A Linear ticket attached to a todo.
#[derive(Debug, Clone, Serialize)]
pub struct TicketLink {
    pub todo_id: TodoId,
    pub key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: TodoId,
    pub depth: usize,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Tree {
    pub todos: Vec<Todo>,
    pub sessions: Vec<SessionLink>,
    pub tickets: Vec<TicketLink>,
}

impl Tree {
    pub fn get(&self, id: TodoId) -> Option<&Todo> {
        self.todos.iter().find(|t| t.id == id)
    }

    pub fn children(&self, parent: Option<TodoId>) -> Vec<&Todo> {
        let mut kids: Vec<&Todo> = self.todos.iter().filter(|t| t.parent_id == parent).collect();
        kids.sort_by_key(|t| (t.position, t.id));
        kids
    }

    pub fn has_children(&self, id: TodoId) -> bool {
        self.todos.iter().any(|t| t.parent_id == Some(id))
    }

    pub fn session_for(&self, id: TodoId) -> Option<&SessionLink> {
        self.sessions.iter().find(|s| s.todo_id == id)
    }

    pub fn tickets_for(&self, id: TodoId) -> Vec<&str> {
        self.tickets.iter().filter(|t| t.todo_id == id).map(|t| t.key.as_str()).collect()
    }

    /// True if `id` is `ancestor` or sits anywhere beneath it.
    pub fn is_within(&self, id: TodoId, ancestor: TodoId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.get(c).and_then(|t| t.parent_id);
        }
        false
    }

    /// Working dir for new sessions: the todo's own, else the nearest ancestor's.
    pub fn dir_for(&self, id: TodoId) -> Option<&str> {
        let mut cur = self.get(id);
        while let Some(t) = cur {
            if let Some(d) = &t.dir {
                return Some(d);
            }
            cur = t.parent_id.and_then(|p| self.get(p));
        }
        None
    }

    /// Titles from root down to `id`, e.g. "Lattice removal › Rebase #1935".
    pub fn path(&self, id: TodoId) -> String {
        let mut parts = Vec::new();
        let mut cur = self.get(id);
        while let Some(t) = cur {
            parts.push(t.title.as_str());
            cur = t.parent_id.and_then(|p| self.get(p));
        }
        parts.reverse();
        parts.join(" › ")
    }

    /// Done todos, most recently checked off first.
    pub fn done_rows(&self) -> Vec<Row> {
        let mut done: Vec<(chrono::DateTime<chrono::FixedOffset>, &Todo)> = self
            .todos
            .iter()
            .filter_map(|t| Some((chrono::DateTime::parse_from_rfc3339(t.done_at.as_deref()?).ok()?, t)))
            .collect();
        done.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.id.cmp(&a.1.id)));
        done.into_iter().map(|(_, t)| Row { id: t.id, depth: 0 }).collect()
    }

    /// Open todos with a linked session, most recently active first. Undated sessions sink to the bottom.
    pub fn active_rows<T: Ord>(&self, last_active: &dyn Fn(&SessionLink) -> Option<T>) -> Vec<Row> {
        let mut active: Vec<(Option<T>, TodoId)> = self
            .todos
            .iter()
            .filter(|t| !t.is_done())
            .filter_map(|t| Some((last_active(self.session_for(t.id)?), t.id)))
            .collect();
        active.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        active.into_iter().map(|(_, id)| Row { id, depth: 0 }).collect()
    }

    /// Depth-first visible rows. A hidden todo hides its whole subtree.
    pub fn rows(&self, collapsed: &HashSet<TodoId>, hidden: &dyn Fn(&Todo) -> bool) -> Vec<Row> {
        let mut out = Vec::new();
        self.walk(None, 0, collapsed, hidden, &mut out);
        out
    }

    fn walk(&self, parent: Option<TodoId>, depth: usize, collapsed: &HashSet<TodoId>, hidden: &dyn Fn(&Todo) -> bool, out: &mut Vec<Row>) {
        for t in self.children(parent) {
            if hidden(t) {
                continue;
            }
            out.push(Row { id: t.id, depth });
            if !collapsed.contains(&t.id) {
                self.walk(Some(t.id), depth + 1, collapsed, hidden, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(id: TodoId, done_at: Option<&str>) -> Todo {
        Todo { id, parent_id: None, position: id, title: id.to_string(), dir: None, done_at: done_at.map(str::to_string), created_at: String::new() }
    }

    #[test]
    fn done_rows_newest_first() {
        let tree = Tree {
            todos: vec![
                todo(1, Some("2026-10-01T09:00:00-04:00")),
                todo(2, None),
                todo(3, Some("2026-10-03T09:00:00-04:00")),
                todo(4, Some("2026-10-03T13:30:00+01:00")),
            ],
            ..Default::default()
        };
        let ids: Vec<TodoId> = tree.done_rows().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![3, 4, 1]);
    }

    #[test]
    fn active_rows_open_claimed_by_recency() {
        let link = |todo_id, sid: &str| SessionLink { todo_id, session_id: sid.into(), cwd: String::new(), linked_at: String::new() };
        let tree = Tree {
            todos: vec![todo(1, None), todo(2, None), todo(3, Some("2026-10-03T09:00:00-04:00")), todo(4, None), todo(5, None)],
            sessions: vec![link(1, "a"), link(2, "b"), link(3, "c"), link(5, "e")],
            ..Default::default()
        };
        let when = |l: &SessionLink| match l.session_id.as_str() {
            "a" => Some(10),
            "b" => Some(30),
            "c" => Some(40),
            _ => None,
        };
        let ids: Vec<TodoId> = tree.active_rows(&when).iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![2, 1, 5]);
    }
}
