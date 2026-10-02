use serde::Serialize;
use std::collections::HashSet;

pub type TodoId = i64;

#[derive(Debug, Clone, Serialize)]
pub struct Todo {
    pub id: TodoId,
    pub parent_id: Option<TodoId>,
    pub position: i64,
    pub title: String,
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

    pub fn sessions_for(&self, id: TodoId) -> Vec<&SessionLink> {
        let mut s: Vec<&SessionLink> = self.sessions.iter().filter(|s| s.todo_id == id).collect();
        s.sort_by(|a, b| b.linked_at.cmp(&a.linked_at));
        s
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
