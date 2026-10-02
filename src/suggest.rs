//! After a reflection: ask Claude which open todos look finished. Suggestions only; the user decides.

use crate::domain::{TodoId, Tree};
use crate::sessions::{self, NameCache};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::SystemTime;

#[derive(Debug, Deserialize)]
pub struct Suggestions {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub done: Vec<Suggestion>,
}

#[derive(Debug, Deserialize)]
pub struct Suggestion {
    pub id: TodoId,
    #[serde(default)]
    pub reason: String,
}

pub fn build_prompt(tree: &Tree, reflection: &str, since: SystemTime) -> String {
    let mut names = NameCache::default();
    let mut todos = String::new();
    for row in tree.rows(&HashSet::new(), &|t| t.is_done()) {
        let t = tree.get(row.id).unwrap();
        let sessions: Vec<String> = tree.sessions_for(t.id).iter().map(|s| names.get(&s.session_id).to_string()).collect();
        let tickets = tree.tickets_for(t.id);
        todos.push_str(&format!("{}#{} {}", "  ".repeat(row.depth), t.id, t.title));
        if !sessions.is_empty() {
            todos.push_str(&format!("  [sessions: {}]", sessions.join(", ")));
        }
        if !tickets.is_empty() {
            todos.push_str(&format!("  [tickets: {}]", tickets.join(", ")));
        }
        todos.push('\n');
    }
    let mut activity = String::new();
    for a in sessions::recent_activity(since, 15) {
        activity.push_str(&format!("## {} ({})\n", a.name, &a.session_id[..8.min(a.session_id.len())]));
        if let Some(j) = a.job {
            activity.push_str(&format!("job status: {j}\n"));
        }
        for r in a.recent {
            activity.push_str(&format!("- {}\n", r.replace('\n', " ")));
        }
    }
    format!(
        "The user keeps a todo tree and just wrote a reflection on their last work block. \
Your only job: point out OPEN todos that now look finished, based on the reflection and session activity. \
Do not propose new todos, renames, or anything else. Be conservative; only clear evidence counts.\n\n\
Reply with JSON only, no prose, no code fence:\n\
{{\"message\": \"one short line\", \"done\": [{{\"id\": <todo id>, \"reason\": \"short evidence\"}}]}}\n\
If nothing looks finished, return an empty done list and message \"All in sync.\"\n\n\
# Reflection\n{reflection}\n\n# Open todos\n{todos}\n# Recent Claude session activity\n{activity}"
    )
}

pub fn run(prompt: &str) -> Result<Suggestions> {
    let mut child = Command::new("claude")
        .args(["-p", "--no-session-persistence", "--model", "sonnet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run claude -p")?;
    child.stdin.take().unwrap().write_all(prompt.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("claude -p failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

fn parse(raw: &str) -> Result<Suggestions> {
    let (Some(a), Some(b)) = (raw.find('{'), raw.rfind('}')) else { bail!("no JSON in reply: {raw}") };
    Ok(serde_json::from_str(&raw[a..=b])?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_fenced_reply() {
        let s = super::parse("```json\n{\"message\":\"x\",\"done\":[{\"id\":3,\"reason\":\"merged\"}]}\n```").unwrap();
        assert_eq!(s.done[0].id, 3);
    }
}
