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
    Timer,
}

enum Mode {
    Normal,
    Input { kind: InputKind, buf: String },
    ConfirmDelete(TodoId),
    PickSession { links: Vec<SessionLink>, idx: usize },
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
            Mode::PickSession { links, mut idx } => match k.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    idx = (idx + 1).min(links.len() - 1);
                    Mode::PickSession { links, idx }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    idx = idx.saturating_sub(1);
                    Mode::PickSession { links, idx }
                }
                KeyCode::Char('d') => {
                    let l = &links[idx];
                    let r = self.store.unlink_session(l.todo_id, &l.session_id);
                    self.act(r, Some(l.todo_id));
                    Mode::Normal
                }
                KeyCode::Enter | KeyCode::Char('o') => {
                    self.open_session(&links[idx].clone(), k.code != KeyCode::Enter, terminal);
                    return;
                }
                _ => Mode::Normal,
            },
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

    fn open_session(&mut self, link: &SessionLink, jump: bool, terminal: &mut DefaultTerminal) {
        let name = self.names.get(&link.session_id).to_string();
        if sessions::in_tmux() {
            self.status = match sessions::tmux_open(&link.session_id, &link.cwd, &name, jump) {
                Ok((at, false)) => format!("Opened “{name}” in {at}."),
                Ok((at, true)) => format!("“{name}” already open in {at}."),
                Err(e) => format!("couldn't open session: {e}"),
            };
            return;
        }
        let mut cmd = sessions::open_command(&link.session_id, &link.cwd);
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
            KeyCode::Char('a') => self.mode = Mode::Input { kind: InputKind::Sibling, buf: String::new() },
            KeyCode::Char('A') => self.mode = Mode::Input { kind: InputKind::Child, buf: String::new() },
            KeyCode::Char('t') => self.mode = Mode::Input { kind: InputKind::Timer, buf: "25".into() },
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
                self.mode = Mode::Input { kind: InputKind::Edit(id), buf };
            }
            KeyCode::Char('d') => self.mode = Mode::ConfirmDelete(id),
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
            KeyCode::Enter | KeyCode::Char('o') => {
                let links: Vec<SessionLink> = self.tree.sessions_for(id).into_iter().cloned().collect();
                match links.len() {
                    0 => self.status = "Not attached to a session. Run /claim inside one.".into(),
                    1 => self.open_session(&links[0], k.code != KeyCode::Enter, terminal),
                    _ => self.mode = Mode::PickSession { links, idx: 0 },
                }
            }
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
            Mode::Input { kind, buf } => {
                let label = match kind {
                    InputKind::Sibling => "new todo",
                    InputKind::Child => "new child todo",
                    InputKind::Edit(_) => "edit",
                    InputKind::Timer => "timer minutes",
                };
                popup(f, label, vec![Line::from(format!("{buf}▏"))], 70, 3);
            }
            Mode::ConfirmDelete(id) => {
                let n = self.tree.todos.iter().filter(|t| t.id != *id && self.tree.is_within(t.id, *id)).count();
                let extra = if n > 0 { format!(" and {n} nested") } else { String::new() };
                let title = self.tree.get(*id).map(|t| t.title.clone()).unwrap_or_default();
                popup(f, "delete", vec![Line::from(format!("Delete “{title}”{extra}? y / n"))], 70, 3);
            }
            Mode::PickSession { links, idx } => {
                let lines: Vec<Line> = links
                    .iter()
                    .enumerate()
                    .map(|(i, l)| {
                        let name = self.names.get(&l.session_id).to_string();
                        let s = format!("{} {}  ({})", if i == *idx { "›" } else { " " }, name, &l.session_id[..8.min(l.session_id.len())]);
                        Line::from(s)
                    })
                    .chain([Line::from(""), Line::from("⏎ open · o open + jump · d detach · esc cancel").style(Style::default().fg(Color::DarkGray))])
                    .collect();
                let h = lines.len() as u16 + 2;
                popup(f, "sessions", lines, 70, h);
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
        let links = self.tree.sessions_for(t.id).into_iter().cloned().collect::<Vec<_>>();
        if let Some(first) = links.first() {
            let more = if links.len() > 1 { format!(" +{}", links.len() - 1) } else { String::new() };
            let name = self.names.get(&first.session_id).to_string();
            spans.push(Span::styled(format!("  ⇢ {name}{more}"), Style::default().fg(Color::Magenta)));
        }
        for key in self.tree.tickets_for(t.id) {
            spans.push(Span::styled(format!("  {key}"), Style::default().fg(Color::Cyan)));
        }
        Line::from(spans)
    }

    fn detail_lines(&mut self) -> Vec<Line<'static>> {
        let Some(id) = self.selected() else { return vec![] };
        let mut lines = vec![Line::from(Span::styled(self.tree.path(id), Style::default().add_modifier(Modifier::BOLD)))];
        let links = self.tree.sessions_for(id).into_iter().cloned().collect::<Vec<_>>();
        if let Some(l) = links.first() {
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
Tab         nest under item above
Shift-Tab   un-nest
J / K       move down / up among siblings
h/l ←/→     collapse / expand
m then p    move item inside another (P = after it)
⏎           open its Claude session (tmux: background window)
o           open and jump to it

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
