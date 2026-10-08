# tt

Todo tree linked to Claude Code sessions. Nest todos, start or resume a Claude session from any of them, and have Claude suggest check-offs after you reflect on a work block.

## Install

Needs Rust (toolchain pinned in `rust-toolchain.toml`; rustup picks it up) and the `claude` CLI on `PATH`.

```sh
cargo install --path .
```

Puts `tt` in `~/.cargo/bin`. Data lives in `~/.local/share/tt/tt.db` (SQLite); set `TT_HOME` to move it.

Optional:

- `tmux`: inside tmux, sessions open in their own window of a dedicated tmux session (`$TT_TMUX_SESSION`, default `claude`) instead of taking over the terminal.
- `terminal-notifier`: timer notifications. Falls back to `osascript`.

### `/claim` in Claude Code

`claude-mod/` is a Claude Code plugin adding `/claim`, a picker pane that links a todo to the current session. Symlink it into your skills dir:

```sh
ln -s "$PWD/claude-mod" ~/.claude/skills/tt-claim
```

New sessions load it. The plugin calls `tt` from `PATH`.

## TUI

Run `tt` with no arguments. `?` shows all keys.

| Key | Action |
| --- | --- |
| `t` / `c` / `p` | views: todos, recently completed, in progress |
| `j` `k` `g` `G` | move, top, bottom |
| `space` | toggle done |
| `a` / `A` | add below / add as child |
| `e` / `x` | edit title / delete |
| `Tab` / `Shift-Tab` | nest / un-nest |
| `J` / `K` | move among siblings |
| `h` / `l` | collapse / expand |
| `d` | directory new sessions start in (inherited by children) |
| `o` | open the todo's Claude session, or start one |
| `u` | detach its session |
| `s` / `S` | start / stop timer |
| `r` | reflect now |
| `H` | show older done items |
| `q` | quit |

`o` on an unclaimed todo starts a new Claude session in the todo's directory with the prompt `Let's work on todo #<id> from tt: <path>`, and links it.

### Timer and reflections

`s` starts a timer (default 25 min). When it ends you get a notification and `$EDITOR` opens a reflection. After you save, `claude -p` reads the reflection plus recent session activity and suggests open todos that look done. You pick which to check off. Reflections are saved under `~/.local/share/tt/reflections/`.

## CLI

Commands taking a session default to `$CLAUDE_CODE_SESSION_ID`, so they work from inside Claude Code; pass `--session <id>` elsewhere.

```sh
tt list [--all] [--json]            # open tree with ids
tt add "title" [--parent ID | --after ID] [--claim]
tt claim ID                         # link todo to this session
tt unclaim ID
tt claimed                          # todos this session claimed
tt ticket ID KEY                    # attach Linear ticket, e.g. ENG-123
tt done ID
tt dir ID [PATH | --clear]          # session start dir (default: cwd)
```
