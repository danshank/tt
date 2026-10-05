use crate::domain::{Row, SessionLink, Todo, TodoId, Tree};
use crate::sessions::{self, NameCache};
use crate::store::{data_dir, Store};
use crate::suggest::{self, Suggestion, Suggestions};
use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::collections::HashSet;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime};

enum InputKind {
    Sibling,
    Child,
    Edit(TodoId),
    Dir(TodoId),
    Timer,
}

impl InputKind {
    fn label(&self) -> &'static str {
        match self {
            InputKind::Sibling => "new todo",
            InputKind::Child => "new child todo",
            InputKind::Edit(_) => "edit",
            InputKind::Dir(_) => "directory (empty clears)",
            InputKind::Timer => "timer minutes",
        }
    }
}

enum Mode {
    Normal,
    Input { kind: InputKind, buf: String },
    ConfirmDelete(TodoId),
    Suggest { message: String, items: Vec<(Suggestion, bool)>, idx: usize },
    Help,
}

struct Timer {
    minutes: f64,
    ends: Instant,
}

struct App {
    store: Store,
    tree: Tree,
    rows: Vec<Row>,
    cursor: usize,
    collapsed: HashSet<TodoId>,
    show_all_done: bool,
    moving: Option<TodoId>,
    mode: Mode,
    timer: Option<Timer>,
    block_start: DateTime<Local>,
    pending: Option<Receiver<Result<Suggestions>>>,
    names: NameCache,
    status: String,
    last_reload: Instant,
    quit: bool,
}

pub fn run(store: Store) -> Result<()> {
    let mut app = App {
        store,
        tree: Tree::default(),
        rows: vec![],
        cursor: 0,
        collapsed: HashSet::new(),
        show_all_done: false,
        moving: None,
        mode: Mode::Normal,
        timer: None,
        block_start: Local::now(),
        pending: None,
        names: NameCache::default(),
        status: "? for keys".into(),
        last_reload: Instant::now(),
        quit: false,
    };
    app.reload();
    let mut terminal = ratatui::init();
    let res = app.event_loop(&mut terminal);
    ratatui::restore();
    res
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Leave the TUI, run `cmd` in the real terminal, come back.
fn suspend(terminal: &mut DefaultTerminal, cmd: &mut Command) -> std::io::Result<std::process::ExitStatus> {
    ratatui::restore();
    let status = cmd.status();
    *terminal = ratatui::init();
    let _ = terminal.clear();
    status
}

fn expand_home(path: &str) -> String {
    match (path.strip_prefix('~'), std::env::var("HOME")) {
        (Some(rest), Ok(home)) if rest.is_empty() || rest.starts_with('/') => format!("{home}{rest}"),
        _ => path.to_string(),
    }
}

/// Split a partly typed path into the part up to the last '/' and the subdirectories there matching the rest.
/// Hidden dirs only show once the typed name starts with '.'.
fn dir_matches(buf: &str) -> (String, Vec<String>) {
    let (head, prefix) = match buf.rfind('/') {
        Some(i) => buf.split_at(i + 1),
        None => ("", buf),
    };
    let base = if head.is_empty() { ".".to_string() } else { expand_home(head) };
    let mut names: Vec<String> = std::fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(prefix) && (prefix.starts_with('.') || !n.starts_with('.')))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    (head.to_string(), names)
}

fn notify(message: &str) {
    let bundle = std::env::var("__CFBundleIdentifier").unwrap_or_default();
    let tn = Command::new("terminal-notifier")
        .args(["-title", "tt", "-message", message, "-sound", "Glass"])
        .args(if bundle.is_empty() { vec![] } else { vec!["-activate".to_string(), bundle] })
        .output();
    if tn.is_err() {
        let script = format!("display notification {:?} with title \"tt\" sound name \"Glass\"", message);
        let _ = Command::new("osascript").args(["-e", &script]).output();
    }
}

impl App {
    fn selected(&self) -> Option<TodoId> {
        self.rows.get(self.cursor).map(|r| r.id)
    }

    fn reload(&mut self) {
        let keep = self.selected();
        match self.store.load() {
            Ok(t) => self.tree = t,
            Err(e) => self.status = format!("load failed: {e}"),
        }
        self.recompute(keep);
        self.last_reload = Instant::now();
    }

    fn recompute(&mut self, keep: Option<TodoId>) {
        let today = today();
        let show_all = self.show_all_done;
        let hidden = move |t: &Todo| !show_all && t.done_at.as_deref().is_some_and(|d| !d.starts_with(&today));
        self.rows = self.tree.rows(&self.collapsed, &hidden);
        if let Some(id) = keep {
            if let Some(i) = self.rows.iter().position(|r| r.id == id) {
                self.cursor = i;
            }
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    fn act(&mut self, r: Result<()>, keep: Option<TodoId>) {
        if let Err(e) = r {
            self.status = e.to_string();
        }
        self.reload();
        self.recompute(keep);
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.on_key(k, terminal);
                    }
                }
            }
            self.tick(terminal);
        }
        Ok(())
    }

    fn tick(&mut self, terminal: &mut DefaultTerminal) {
        if matches!(self.mode, Mode::Normal) && self.last_reload.elapsed() > Duration::from_secs(2) {
            self.reload();
        }
        if self.timer.as_ref().is_some_and(|t| Instant::now() >= t.ends) {
            let minutes = self.timer.take().map(|t| t.minutes).unwrap_or(0.0);
            notify(&format!("{minutes} min up. What happened?"));
            self.reflect(terminal);
        }
        if matches!(self.mode, Mode::Normal) {
            if let Some(rx) = &self.pending {
                if let Ok(res) = rx.try_recv() {
                    self.pending = None;
                    self.show_suggestions(res);
                }
            }
        }
    }

    fn show_suggestions(&mut self, res: Result<Suggestions>) {
        match res {
            Err(e) => self.status = format!("suggestions failed: {e}"),
            Ok(s) => {
                let items: Vec<(Suggestion, bool)> = s
                    .done
                    .into_iter()
                    .filter(|d| self.tree.get(d.id).is_some_and(|t| !t.is_done()))
                    .map(|d| (d, true))
                    .collect();
                if items.is_empty() {
                    self.status = "All in sync.".into();
                } else {
                    self.mode = Mode::Suggest { message: s.message, items, idx: 0 };
                }
            }
        }
    }

    fn reflect(&mut self, terminal: &mut DefaultTerminal) {
        let start = self.block_start;
        let end = Local::now();
        let dir = data_dir().join("reflections");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}.md", end.format("%Y-%m-%d-%H%M")));
        let header = format!(
            "# {} – {} · What happened?\n# Lines starting with # are ignored. Save and quit when done.\n\n",
            start.format("%H:%M"),
            end.format("%H:%M")
        );
        if std::fs::write(&path, header).is_err() {
            self.status = "couldn't write reflection file".into();
            return;
        }
        let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
        let mut parts = editor.split_whitespace();
        let mut cmd = Command::new(parts.next().unwrap_or("vi"));
        cmd.args(parts).arg(&path);
        if let Err(e) = suspend(terminal, &mut cmd) {
            self.status = format!("editor failed: {e}");
            return;
        }
        let body: String = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        if body.is_empty() {
            let _ = std::fs::remove_file(&path);
            self.status = "Empty reflection, nothing saved.".into();
            return;
        }
        let _ = self.store.add_reflection(&start.to_rfc3339(), &end.to_rfc3339(), &body);
        self.block_start = end;
        self.reload();
        let prompt = suggest::build_prompt(&self.tree, &body, SystemTime::from(start));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(suggest::run(&prompt));
        });
        self.pending = Some(rx);
        self.status = "Reflection saved. Checking todos against your sessions…".into();
    }

    fn on_key(&mut self, k: KeyEvent, terminal: &mut DefaultTerminal) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        self.mode = match mode {
            Mode::Normal => {
                self.on_normal(k, terminal);
                return;
            }
            Mode::Help => Mode::Normal,
            Mode::Input { kind, mut buf } => match k.code {
                KeyCode::Esc => Mode::Normal,
                KeyCode::Enter => {
                    self.submit(kind, buf);
                    return;
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Mode::Input { kind, buf }
                }
                KeyCode::Tab if matches!(kind, InputKind::Dir(_)) => {
                    let (head, names) = dir_matches(&buf);
                    if let [only] = names.as_slice() {
                        buf = format!("{head}{only}/");
                    } else if let Some(first) = names.first() {
                        let lcp = names.iter().fold(first.as_str(), |acc, n| {
                            let len = acc.char_indices().zip(n.chars()).take_while(|((_, a), b)| a == b).last().map_or(0, |((i, c), _)| i + c.len_utf8());
                            &acc[..len]
                        });
                        buf = format!("{head}{lcp}");
                    }
                    Mode::Input { kind, buf }
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    Mode::Input { kind, buf }
                }
                _ => Mode::Input { kind, buf },
            },
            Mode::ConfirmDelete(id) => {
                if k.code == KeyCode::Char('y') {
                    let r = self.store.delete(id);
                    self.act(r, None);
                }
                Mode::Normal
            }
            Mode::Suggest { message, mut items, mut idx } => match k.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    idx = (idx + 1).min(items.len() - 1);
                    Mode::Suggest { message, items, idx }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    idx = idx.saturating_sub(1);
                    Mode::Suggest { message, items, idx }
                }
                KeyCode::Char(' ') => {
                    items[idx].1 = !items[idx].1;
                    Mode::Suggest { message, items, idx }
                }
                KeyCode::Enter => {
                    let ids: Vec<TodoId> = items.iter().filter(|(_, on)| *on).map(|(s, _)| s.id).collect();
                    for id in &ids {
                        let _ = self.store.set_done(*id, true);
                    }
                    self.status = format!("Checked off {}.", ids.len());
                    self.reload();
                    Mode::Normal
                }
                _ => {
                    self.status = "Suggestions dismissed.".into();
                    Mode::Normal
                }
            },
        };
    }

    fn prompt(&mut self, kind: InputKind, buf: String) {
        if !sessions::in_tmux() || matches!(kind, InputKind::Dir(_)) {
            self.mode = Mode::Input { kind, buf };
            return;
        }
        match sessions::tmux_edit(kind.label(), &buf) {
            Ok(Some(text)) => self.submit(kind, text),
            Ok(None) => {}
            Err(e) => {
                self.status = format!("editor popup failed: {e}");
                self.mode = Mode::Input { kind, buf };
            }
        }
    }

    fn submit(&mut self, kind: InputKind, buf: String) {
        let cur = self.selected();
        match kind {
            InputKind::Timer => match buf.trim().parse::<f64>() {
                Ok(m) if m > 0.0 => {
                    self.timer = Some(Timer { minutes: m, ends: Instant::now() + Duration::from_secs_f64(m * 60.0) });
                    self.status = format!("Timer set: {m} min.");
                }
                _ => self.status = "Timer needs a number of minutes.".into(),
            },
            InputKind::Edit(id) => {
                let r = self.store.rename(id, &buf);
                self.act(r, Some(id));
            }
            InputKind::Dir(id) => {
                let dir = expand_home(buf.trim());
                let dir = if dir.len() > 1 { dir.trim_end_matches('/').to_string() } else { dir };
                if !dir.is_empty() && !std::path::Path::new(&dir).is_dir() {
                    self.status = format!("not a directory: {dir}");
                    return;
                }
                let r = self.store.set_dir(id, (!dir.is_empty()).then_some(dir.as_str()));
                self.act(r, Some(id));
            }
            InputKind::Sibling | InputKind::Child => {
                let (parent, index) = match (&kind, cur.and_then(|c| self.tree.get(c))) {
                    (_, None) => (None, None),
                    (InputKind::Child, Some(t)) => (Some(t.id), None),
                    (_, Some(t)) => (t.parent_id, Some(t.position as usize + 1)),
                };
                match self.store.add(&buf, parent, index) {
                    Ok(id) => {
                        if let Some(p) = parent {
                            self.collapsed.remove(&p);
                        }
                        self.act(Ok(()), Some(id));
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
        }
    }

    fn open_session(&mut self, link: &SessionLink, terminal: &mut DefaultTerminal) {
        let name = self.names.get(&link.session_id).to_string();
        self.launch(&link.session_id, &sessions::open_args(&link.session_id), &link.cwd, &name, terminal);
    }

    /// New Claude session seeded with the todo, opened in the todo's dir and claimed up front.
    fn start_session(&mut self, id: TodoId, terminal: &mut DefaultTerminal) {
        let Some(todo) = self.tree.get(id) else { return };
        let name: String = todo.title.chars().take(30).collect();
        let cwd = self.tree.dir_for(id).map(str::to_string).unwrap_or_else(|| {
            std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default()
        });
        let tickets = self.tree.tickets_for(id);
        let mut prompt = format!("Let's work on todo #{id} from tt: {}", self.tree.path(id));
        if !tickets.is_empty() {
            prompt.push_str(&format!(" (Linear: {})", tickets.join(", ")));
        }
        let sid = uuid::Uuid::new_v4().to_string();
        if let Err(e) = self.store.link_session(id, &sid, &cwd) {
            self.status = format!("couldn't claim: {e}");
            return;
        }
        self.launch(&sid, &sessions::start_args(&sid, &prompt), &cwd, &name, terminal);
        self.reload();
    }

    fn launch(&mut self, session_id: &str, args: &[String], cwd: &str, name: &str, terminal: &mut DefaultTerminal) {
        if sessions::in_tmux() {
            self.status = match sessions::tmux_open(session_id, args, cwd, name) {
                Ok((at, false)) => format!("Opened “{name}” in {at}."),
                Ok((at, true)) => format!("“{name}” already open in {at}."),
                Err(e) => format!("couldn't open session: {e}"),
            };
            return;
        }
        let mut cmd = sessions::claude_command(args, cwd);
        match suspend(terminal, &mut cmd) {
            Ok(_) => self.status = format!("Back from “{name}”."),
            Err(e) => self.status = format!("couldn't open session: {e}"),
        }
        self.reload();
    }

    fn on_normal(&mut self, k: KeyEvent, terminal: &mut DefaultTerminal) {
        let cur = self.selected();
        let last = self.rows.len().saturating_sub(1);
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('j') | KeyCode::Down => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('g') => self.cursor = 0,
            KeyCode::Char('G') => self.cursor = last,
            KeyCode::Char('a') => self.prompt(InputKind::Sibling, String::new()),
            KeyCode::Char('A') => self.prompt(InputKind::Child, String::new()),
            KeyCode::Char('t') => self.prompt(InputKind::Timer, "25".into()),
            KeyCode::Char('T') => {
                self.timer = None;
                self.status = "Timer stopped.".into();
            }
            KeyCode::Char('r') => self.reflect(terminal),
            KeyCode::Char('H') => {
                self.show_all_done = !self.show_all_done;
                self.recompute(cur);
            }
            KeyCode::Esc => {
                self.moving = None;
                self.status.clear();
            }
            _ => {}
        }
        let Some(id) = cur else { return };
        match k.code {
            KeyCode::Char(' ') | KeyCode::Char('x') => {
                let done = self.tree.get(id).is_some_and(|t| t.is_done());
                let r = self.store.set_done(id, !done);
                self.act(r, Some(id));
            }
            KeyCode::Char('e') => {
                let buf = self.tree.get(id).map(|t| t.title.clone()).unwrap_or_default();
                self.prompt(InputKind::Edit(id), buf);
            }
            KeyCode::Char('d') => self.mode = Mode::ConfirmDelete(id),
            KeyCode::Char('D') => {
                let buf = self.tree.dir_for(id).map(|d| format!("{}/", d.trim_end_matches('/'))).unwrap_or_else(|| "~/".into());
                self.prompt(InputKind::Dir(id), buf);
            }
            KeyCode::Tab => {
                let r = self.store.indent(id);
                if let Some(p) = self.store.load().ok().and_then(|t| t.get(id).and_then(|t| t.parent_id)) {
                    self.collapsed.remove(&p);
                }
                self.act(r, Some(id));
            }
            KeyCode::BackTab => {
                let r = self.store.outdent(id);
                self.act(r, Some(id));
            }
            KeyCode::Char('J') => {
                let r = self.store.shift(id, 1);
                self.act(r, Some(id));
            }
            KeyCode::Char('K') => {
                let r = self.store.shift(id, -1);
                self.act(r, Some(id));
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if self.tree.has_children(id) && !self.collapsed.contains(&id) {
                    self.collapsed.insert(id);
                    self.recompute(Some(id));
                } else if let Some(p) = self.tree.get(id).and_then(|t| t.parent_id) {
                    self.recompute(Some(p));
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.collapsed.remove(&id);
                self.recompute(Some(id));
            }
            KeyCode::Char('m') => {
                self.moving = Some(id);
                self.status = "Moving. Go to target: p = put inside, P = put after, Esc = cancel.".into();
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                if let Some(m) = self.moving.take() {
                    let r = if k.code == KeyCode::Char('p') {
                        self.collapsed.remove(&id);
                        self.store.place(m, Some(id), usize::MAX)
                    } else {
                        let t = self.tree.get(id).unwrap();
                        self.store.place(m, t.parent_id, t.position as usize + 1)
                    };
                    self.status.clear();
                    self.act(r, Some(m));
                }
            }
            KeyCode::Char('o') => match self.tree.session_for(id).cloned() {
                Some(link) => self.open_session(&link, terminal),
                None => self.start_session(id, terminal),
            },
            _ => {}
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let [main, detail, bar] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(3), Constraint::Length(1)]).areas(f.area());

        let rows = self.rows.clone();
        let items: Vec<ListItem> = rows.iter().map(|r| ListItem::new(self.row_line(r))).collect();
        let title = if self.show_all_done { " todos (all) " } else { " todos " };
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
        let mut state = ListState::default().with_selected((!self.rows.is_empty()).then_some(self.cursor));
        f.render_stateful_widget(list, main, &mut state);
        if self.rows.is_empty() {
            let hint = Paragraph::new("No todos yet. Press a to add one.").style(Style::default().fg(Color::DarkGray));
            f.render_widget(hint, Rect { x: main.x + 2, y: main.y + 1, width: main.width.saturating_sub(4), height: 1 });
        }

        f.render_widget(Paragraph::new(self.detail_lines()).wrap(Wrap { trim: true }), detail);
        f.render_widget(Paragraph::new(self.bar_line()), bar);

        match &self.mode {
            Mode::Normal => {}
            Mode::Help => popup(f, "keys", HELP.lines().map(Line::from).collect(), 60, 24),
            Mode::Input { kind: kind @ InputKind::Dir(_), buf } => {
                let (_, names) = dir_matches(buf);
                let mut lines = vec![Line::from(format!("{buf}▏"))];
                lines.extend(names.iter().take(8).map(|n| Line::from(format!("  {n}/")).style(Style::default().fg(Color::DarkGray))));
                if names.len() > 8 {
                    lines.push(Line::from(format!("  … {} more", names.len() - 8)).style(Style::default().fg(Color::DarkGray)));
                }
                lines.resize(10, Line::from(""));
                lines.push(Line::from("tab complete · ⏎ save · esc cancel").style(Style::default().fg(Color::DarkGray)));
                popup(f, kind.label(), lines, 70, 13);
            }
            Mode::Input { kind, buf } => {
                popup(f, kind.label(), vec![Line::from(format!("{buf}▏"))], 70, 3);
            }
            Mode::ConfirmDelete(id) => {
                let n = self.tree.todos.iter().filter(|t| t.id != *id && self.tree.is_within(t.id, *id)).count();
                let extra = if n > 0 { format!(" and {n} nested") } else { String::new() };
                let title = self.tree.get(*id).map(|t| t.title.clone()).unwrap_or_default();
                popup(f, "delete", vec![Line::from(format!("Delete “{title}”{extra}? y / n"))], 70, 3);
            }
            Mode::Suggest { message, items, idx } => {
                let mut lines = vec![Line::from(message.clone()), Line::from("")];
                for (i, (s, on)) in items.iter().enumerate() {
                    let mark = if *on { "[x]" } else { "[ ]" };
                    let cur = if i == *idx { "›" } else { " " };
                    lines.push(Line::from(format!("{cur} {mark} {}", self.tree.path(s.id))));
                    lines.push(Line::from(format!("      {}", s.reason)).style(Style::default().fg(Color::DarkGray)));
                }
                lines.push(Line::from(""));
                lines.push(Line::from("space toggle · ⏎ check off marked · esc dismiss").style(Style::default().fg(Color::DarkGray)));
                let h = lines.len() as u16 + 2;
                popup(f, "looks done?", lines, 80, h);
            }
        }
    }

    fn row_line(&mut self, r: &Row) -> Line<'static> {
        let t = self.tree.get(r.id).unwrap().clone();
        let fold = match (self.tree.has_children(t.id), self.collapsed.contains(&t.id)) {
            (false, _) => "  ",
            (true, true) => "▸ ",
            (true, false) => "▾ ",
        };
        let done = t.is_done();
        let mut spans = vec![
            Span::raw("  ".repeat(r.depth)),
            Span::styled(fold, Style::default().fg(Color::DarkGray)),
            Span::styled(if done { "[x] " } else { "[ ] " }, Style::default().fg(if done { Color::Green } else { Color::Reset })),
        ];
        let mut title_style = if done { Style::default().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT) } else { Style::default() };
        if self.moving == Some(t.id) {
            title_style = title_style.fg(Color::Yellow).add_modifier(Modifier::ITALIC);
        }
        spans.push(Span::styled(t.title.clone(), title_style));
        if let Some(link) = self.tree.session_for(t.id).cloned() {
            let name = self.names.get(&link.session_id).to_string();
            spans.push(Span::styled(format!("  ⇢ {name}"), Style::default().fg(Color::Magenta)));
        }
        for key in self.tree.tickets_for(t.id) {
            spans.push(Span::styled(format!("  {key}"), Style::default().fg(Color::Cyan)));
        }
        Line::from(spans)
    }

    fn detail_lines(&mut self) -> Vec<Line<'static>> {
        let Some(id) = self.selected() else { return vec![] };
        let mut first = vec![Span::styled(self.tree.path(id), Style::default().add_modifier(Modifier::BOLD))];
        if let Some(d) = self.tree.dir_for(id) {
            first.push(Span::styled(format!("  in {d}"), Style::default().fg(Color::DarkGray)));
        }
        let mut lines = vec![Line::from(first)];
        if let Some(l) = self.tree.session_for(id).cloned() {
            let status = sessions::job_detail(&l.session_id).unwrap_or_else(|| l.cwd.clone());
            let name = self.names.get(&l.session_id).to_string();
            lines.push(Line::from(Span::styled(format!("⇢ {name}: {status}"), Style::default().fg(Color::DarkGray))));
        }
        lines
    }

    fn bar_line(&self) -> Line<'static> {
        let timer = match &self.timer {
            Some(t) => {
                let left = t.ends.saturating_duration_since(Instant::now()).as_secs();
                Span::styled(format!(" ⏱ {:02}:{:02} ", left / 60, left % 60), Style::default().fg(Color::Black).bg(Color::Yellow))
            }
            None => Span::styled(" ⏱ off ", Style::default().fg(Color::Black).bg(Color::Gray)),
        };
        let since = Span::styled(format!(" since {} ", self.block_start.format("%H:%M")), Style::default().fg(Color::DarkGray));
        let waiting = if self.pending.is_some() { " ⋯ " } else { " " };
        Line::from(vec![timer, since, Span::raw(waiting), Span::raw(self.status.clone())])
    }
}

const HELP: &str = "\
j/k ↑/↓     move            g/G   top / bottom
space / x   toggle done     H     show older done items
a           add below       A     add inside (child)
e           edit title      d     delete
D           set directory for new sessions (children inherit)
Tab         nest under item above
Shift-Tab   un-nest
J / K       move down / up among siblings
h/l ←/→     collapse / expand
m then p    move item inside another (P = after it)
o           open its Claude session, or start one in its directory

t           start a timer (minutes)
T           stop timer
r           reflect now

When the timer ends you get a notification and a
blank reflection opens. After you save it, Claude
checks your sessions and suggests check-offs.

Link sessions from inside Claude with /claim.
q           quit";

fn popup(f: &mut Frame, title: &str, lines: Vec<Line>, width: u16, height: u16) {
    let area = f.area();
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 3, width: w, height: h };
    f.render_widget(Clear, r);
    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(format!(" {title} ")));
    f.render_widget(p, r);
}

#[cfg(test)]
mod tests {
    use super::dir_matches;

    #[test]
    fn completes_subdirs() {
        let root = std::env::temp_dir().join(format!("tt-dirs-{}", std::process::id()));
        for d in ["alpha", "alps", "beta", ".hidden"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("alfile"), "").unwrap();
        let base = format!("{}/", root.display());
        assert_eq!(dir_matches(&format!("{base}al")), (base.clone(), vec!["alpha".into(), "alps".into()]));
        assert_eq!(dir_matches(&base).1, vec!["alpha", "alps", "beta"]);
        assert_eq!(dir_matches(&format!("{base}.h")).1, vec![".hidden"]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
