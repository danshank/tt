//! Read-only view over Claude Code's on-disk sessions (~/.claude).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

pub fn claude_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".claude")
}

pub fn transcript_path(session_id: &str) -> Option<PathBuf> {
    let projects = claude_dir().join("projects");
    std::fs::read_dir(projects).ok()?.flatten().map(|d| d.path().join(format!("{session_id}.jsonl"))).find(|p| p.exists())
}

/// When the session last wrote to its transcript.
pub fn last_active(session_id: &str) -> Option<SystemTime> {
    std::fs::metadata(transcript_path(session_id)?).ok()?.modified().ok()
}

fn short(session_id: &str) -> &str {
    &session_id[..session_id.len().min(8)]
}

fn job_dir(session_id: &str) -> PathBuf {
    claude_dir().join("jobs").join(short(session_id))
}

pub fn is_background_job(session_id: &str) -> bool {
    job_dir(session_id).join("state.json").exists()
}

/// Latest one-line status a background job reported, if any.
pub fn job_detail(session_id: &str) -> Option<String> {
    let raw = std::fs::read_to_string(job_dir(session_id).join("state.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(format!("{} — {}", v["state"].as_str().unwrap_or("?"), v["detail"].as_str().unwrap_or("")))
}

/// Name shown in the agents list (agent-name / ai-title record), falling back to the short id.
pub fn display_name(session_id: &str) -> String {
    transcript_path(session_id)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| {
            raw.lines().rev().find_map(|l| {
                if !(l.contains("\"agent-name\"") || l.contains("\"ai-title\"")) {
                    return None;
                }
                let v: serde_json::Value = serde_json::from_str(l).ok()?;
                v["agentName"].as_str().or(v["aiTitle"].as_str()).map(str::to_string)
            })
        })
        .unwrap_or_else(|| short(session_id).to_string())
}

#[derive(Default)]
pub struct NameCache(HashMap<String, String>);

impl NameCache {
    pub fn get(&mut self, session_id: &str) -> &str {
        self.0.entry(session_id.to_string()).or_insert_with(|| display_name(session_id))
    }
}

/// Args to `claude` that reopen a session: attach for background jobs, resume otherwise.
pub fn open_args(session_id: &str) -> Vec<String> {
    if is_background_job(session_id) {
        vec!["attach".into(), short(session_id).into()]
    } else {
        vec!["--resume".into(), session_id.into()]
    }
}

/// Args to `claude` that start a fresh session with a fixed id and opening prompt.
pub fn start_args(session_id: &str, prompt: &str) -> Vec<String> {
    vec!["--session-id".into(), session_id.into(), prompt.into()]
}

/// Command that runs `claude args` in the current terminal.
pub fn claude_command(args: &[String], cwd: &str) -> Command {
    let mut cmd = Command::new("claude");
    cmd.args(args);
    if Path::new(cwd).is_dir() {
        cmd.current_dir(cwd);
    }
    cmd
}

pub fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

/// Edit `initial` in nvim inside a small tmux popup. None if the user quit without writing.
pub fn tmux_edit(title: &str, initial: &str) -> anyhow::Result<Option<String>> {
    let path = std::env::temp_dir().join(format!("tt-input-{}.txt", std::process::id()));
    std::fs::write(&path, initial)?;
    let before = std::fs::metadata(&path)?.modified()?;
    let nvim = format!(
        "nvim --clean -n -c 'set laststatus=0 noshowmode noruler nonumber norelativenumber signcolumn=no nowrap fillchars=eob:\\  shortmess+=W' \
         -c 'nnoremap <buffer> <CR> <Cmd>silent wq<CR>' -c 'inoremap <buffer> <CR> <Cmd>silent wq<CR>' -c 'startinsert!' '{}'",
        path.display()
    );
    let status = Command::new("tmux").args(["display-popup", "-E", "-x", "C", "-y", "C", "-w", "72", "-h", "5", "-T", &format!(" {title} "), &nvim]).status();
    let saved = std::fs::metadata(&path)?.modified()? != before;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    if !status?.success() || !saved {
        return Ok(None);
    }
    Ok(Some(text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")))
}

fn tmux(args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("tmux").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("tmux {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run `claude args` for a session in its own window of the dedicated tmux session ($TT_TMUX_SESSION, default "claude"),
/// reusing the window if one already runs it, and switch the client there. Returns "session:index".
pub fn tmux_open(session_id: &str, claude_args: &[String], cwd: &str, name: &str) -> anyhow::Result<(String, bool)> {
    let existing = tmux(&["list-windows", "-a", "-F", "#{@tt_session}\t#{window_id}"])?
        .lines()
        .find_map(|l| l.split_once('\t').filter(|(s, _)| *s == session_id).map(|(_, w)| w.to_string()));
    let reused = existing.is_some();
    let window = match existing {
        Some(w) => w,
        None => {
            let target = std::env::var("TT_TMUX_SESSION").unwrap_or_else(|_| "claude".into());
            let cwd = if Path::new(cwd).is_dir() { cwd } else { "." };
            let target_arg = format!("={target}:");
            let mut args = if tmux(&["has-session", "-t", &format!("={target}")]).is_ok() {
                vec!["new-window", "-t", &target_arg]
            } else {
                vec!["new-session", "-s", &target]
            };
            args.extend(["-d", "-P", "-F", "#{window_id}", "-n", name, "-c", cwd, "claude"]);
            args.extend(claude_args.iter().map(String::as_str));
            let w = tmux(&args)?;
            tmux(&["set-option", "-w", "-t", &w, "@tt_session", session_id])?;
            w
        }
    };
    tmux(&["select-window", "-t", &window])?;
    tmux(&["switch-client", "-t", &window])?;
    let label = tmux(&["display-message", "-p", "-t", &window, "#{session_name}:#{window_index}"])?;
    Ok((label, reused))
}

/// What a session has been saying lately: last few assistant text replies.
pub struct Activity {
    pub session_id: String,
    pub name: String,
    pub job: Option<String>,
    pub recent: Vec<String>,
}

pub fn recent_activity(since: SystemTime, limit: usize) -> Vec<Activity> {
    let Ok(projects) = std::fs::read_dir(claude_dir().join("projects")) else { return vec![] };
    let mut files: Vec<(SystemTime, PathBuf)> = projects
        .flatten()
        .filter_map(|d| std::fs::read_dir(d.path()).ok())
        .flat_map(|rd| rd.flatten())
        .filter(|f| f.path().extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|f| Some((f.metadata().ok()?.modified().ok()?, f.path())))
        .filter(|(m, _)| *m >= since)
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .take(limit)
        .filter_map(|(_, p)| {
            let sid = p.file_stem()?.to_str()?.to_string();
            let raw = std::fs::read_to_string(&p).ok()?;
            let mut recent: Vec<String> = raw
                .lines()
                .rev()
                .filter(|l| l.contains("\"type\":\"assistant\""))
                .filter_map(|l| {
                    let v: serde_json::Value = serde_json::from_str(l).ok()?;
                    let text: String = v["message"]["content"]
                        .as_array()?
                        .iter()
                        .filter_map(|c| c["text"].as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    (!text.trim().is_empty()).then(|| text.chars().take(500).collect())
                })
                .take(3)
                .collect();
            recent.reverse();
            Some(Activity { name: display_name(&sid), job: job_detail(&sid), session_id: sid, recent })
        })
        .collect()
}
