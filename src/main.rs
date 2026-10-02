mod domain;
mod sessions;
mod store;
mod suggest;
mod tui;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use domain::TodoId;
use std::collections::HashSet;
use store::Store;

/// Todo tree linked to Claude Code sessions. No subcommand opens the TUI.
#[derive(Parser)]
#[command(name = "tt")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the open todo tree with ids
    List {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Add a todo; --claim also links it to the current Claude session
    Add {
        title: String,
        #[arg(long)]
        parent: Option<TodoId>,
        #[arg(long)]
        claim: bool,
        #[arg(long)]
        session: Option<String>,
    },
    /// Link a todo to the current Claude session (or --session)
    Claim {
        id: TodoId,
        #[arg(long)]
        session: Option<String>,
    },
    /// Detach a todo from the current Claude session (or --session)
    Unclaim {
        id: TodoId,
        #[arg(long)]
        session: Option<String>,
    },
    /// Todos claimed by the current Claude session (or --session)
    Claimed {
        #[arg(long)]
        session: Option<String>,
    },
    /// Attach a Linear ticket key to a todo
    Ticket { id: TodoId, key: String },
    /// Mark a todo done
    Done { id: TodoId },
}

fn session_id(arg: Option<String>) -> Result<String> {
    match arg.or_else(|| std::env::var("CLAUDE_CODE_SESSION_ID").ok()) {
        Some(s) if !s.is_empty() => Ok(s),
        _ => bail!("no session: run inside Claude Code or pass --session"),
    }
}

fn cwd() -> String {
    std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default()
}

fn print_tree(store: &Store, all: bool) -> Result<()> {
    let tree = store.load()?;
    let mut names = sessions::NameCache::default();
    let rows = tree.rows(&HashSet::new(), &|t| !all && t.is_done());
    if rows.is_empty() {
        println!("(no open todos)");
    }
    for r in rows {
        let t = tree.get(r.id).unwrap();
        let mut line = format!("{:>4}  {}{} {}", t.id, "  ".repeat(r.depth), if t.is_done() { "[x]" } else { "[ ]" }, t.title);
        let s: Vec<String> = tree.sessions_for(t.id).iter().map(|l| names.get(&l.session_id).to_string()).collect();
        if !s.is_empty() {
            line.push_str(&format!("  ⇢ {}", s.join(", ")));
        }
        let k = tree.tickets_for(t.id);
        if !k.is_empty() {
            line.push_str(&format!("  {}", k.join(" ")));
        }
        println!("{line}");
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open_default()?;
    match cli.cmd {
        None => tui::run(store)?,
        Some(Cmd::List { all, json }) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&store.load()?)?);
            } else {
                print_tree(&store, all)?;
            }
        }
        Some(Cmd::Add { title, parent, claim, session }) => {
            let id = store.add(&title, parent, None)?;
            if claim {
                store.link_session(id, &session_id(session)?, &cwd())?;
            }
            println!("added #{id}: {}", store.load()?.path(id));
        }
        Some(Cmd::Claim { id, session }) => {
            store.link_session(id, &session_id(session)?, &cwd())?;
            println!("claimed #{id}: {}", store.load()?.path(id));
        }
        Some(Cmd::Unclaim { id, session }) => {
            store.unlink_session(id, &session_id(session)?)?;
            println!("unclaimed #{id}");
        }
        Some(Cmd::Claimed { session }) => {
            let sid = session_id(session)?;
            let tree = store.load()?;
            let mine: Vec<_> = tree.sessions.iter().filter(|l| l.session_id == sid).collect();
            if mine.is_empty() {
                println!("(nothing claimed by this session)");
            }
            for l in mine {
                let t = tree.get(l.todo_id).unwrap();
                let k = tree.tickets_for(t.id);
                println!("{:>4}  {} {}  {}", t.id, if t.is_done() { "[x]" } else { "[ ]" }, tree.path(t.id), k.join(" "));
            }
        }
        Some(Cmd::Ticket { id, key }) => {
            store.link_ticket(id, &key)?;
            println!("attached {} to #{id}", key.to_uppercase());
        }
        Some(Cmd::Done { id }) => {
            store.set_done(id, true)?;
            println!("done #{id}");
        }
    }
    Ok(())
}
