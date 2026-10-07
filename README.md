# dox

A focused Docker TUI for [colima](https://github.com/abiosoft/colima) — lazydocker, but smaller.
Just the parts I actually use: list containers / images / volumes / networks,
stop or delete them, and tail container logs with copy-to-clipboard.

Built in Rust with [ratatui](https://ratatui.rs/) and [bollard](https://github.com/fussybeaver/bollard).
Talks to the Docker daemon directly over its unix socket — no shelling out to `docker`.

## Why

Most Docker TUIs, lazydocker included, shell out to the `docker` CLI for every
action: spawn a process, parse its text output, repeat on every refresh tick.
`dox` talks to the daemon directly over the Docker API via `bollard`, so
listing containers or tailing logs doesn't round-trip through a subprocess.

It also resolves colima's socket (`~/.colima/default/docker.sock`) before
falling back to the system default, so it works out of the box on a colima
setup — most general-purpose TUIs assume Docker Desktop's socket and need
`DOCKER_HOST` set by hand to work with colima.

Log scrollback and copying actually work. You can page/arrow back through
history without losing your place, independent of live tail-follow — see
[Copying log text](#copying-log-text) for the three ways to get text out of
the pane. A lot of terminal Docker TUIs either lock you to tail-only or fight
you on copy once mouse capture is on.

Containers group by compose project automatically (`com.docker.compose.project`,
the label `docker compose` already sets — no extra API calls). Press `z` to
fold a stack you're not touching right now; a flat list gets hard to scan once
a handful of stacks are running side by side. Inside a group, names drop the
`<project>-` prefix compose adds, since the header already shows it.

The PORTS column shows what's actually published (`docker ps` notation:
`hostport->containerport/proto`, or bare `port/proto` when it's exposed but
not published) — deduped, so a port bound on both IPv4 and IPv6 shows once,
not twice. Selecting a container's logs shows its full mapping list in the
pane title, in case it doesn't fit the column.

Running containers show live CPU, MEM, NET I/O, BLOCK I/O and PIDS columns —
the numbers `docker stats` (and [ctop](https://github.com/bcicen/ctop)) show.
dox keeps one stats stream open per running container and closes it when the
container stops. MEM leaves out reclaimable page cache, as `docker stats` does.
The columns appear only when the Containers panel has room, and drop off from
the right (PIDS first) as it narrows.

The log pane follows whatever's highlighted, debounced by ~200ms — scan
quickly down a long list and only the container you actually stop on ever
opens a stream, not every row you passed through. `Enter` / `l` switch
immediately if you don't want to wait out the debounce.

And it deliberately does less. No exec-into-container shell, no CPU/memory
graphs — see [What's out of scope](#whats-out-of-scope-intentional). If you
just need to see what's running, stop or delete it, and read its logs, that's
the whole tool: no feature surface to dig through.

## Install

```sh
brew install pkumar2026/dox/dox
```

Or build from source:

```sh
git clone https://github.com/pkumar2026/dox
cd dox
cargo install --path .
```

`~/.cargo/bin` should already be on `$PATH`. Run with `dox`.

## Use

```
dox                            # boot the TUI
dox --host unix:///path.sock   # override daemon endpoint
dox --refresh-ms 500           # poll lists twice a second
dox --mouse                    # enable wheel scrolling (disables native selection)
dox --list-containers          # non-interactive smoke test
```

### Copying log text

Mouse capture is **on by default** (wheel scrolls the logs). Three ways to copy:

1. **Click + drag in the log pane**, then press `y`. The drag selects log lines
   (highlighted blue); `y` copies them to the system clipboard. This is the
   lazygit / lazydocker pattern and works everywhere.
2. **Hold Shift while click-dragging** for native terminal selection. Alacritty,
   Ghostty, Kitty, iTerm2, and WezTerm all bypass mouse capture when Shift is
   held. Then ⌘C (macOS) or Ctrl-Shift-C (Linux) copies natively.
3. **Keyboard-only**: press `v` to enter visual mode, `↑`/`↓` (or `J`/`K`) to
   extend the selection, `y` to copy.

Press `m` to toggle mouse capture off entirely if you prefer to drag-select
without holding Shift — the trade-off is you lose wheel scrolling.

By default `dox` looks for a daemon in this order:

1. `--host` flag
2. `DOCKER_HOST` env var
3. the active `docker context` socket
4. `~/.colima/default/docker.sock`
5. `/var/run/docker.sock`

## Keys

| key                  | action                                                                    |
|----------------------|---------------------------------------------------------------------------|
| `Tab` / `Shift-Tab`  | cycle Containers → Images → Volumes → Networks                            |
| `↑` `↓`              | move selection                                                            |
| `g` / `G`            | jump to top / bottom                                                      |
| `Enter`              | focus the detail (logs) pane                                              |
| `Esc`                | back to list / dismiss modal                                              |
| `x` / `s` / `r`      | stop / start / restart container                                          |
| `d`                  | delete selected (with confirm)                                            |
| `D`                  | prune dangling images / unused volumes / unused networks                  |
| `z`                  | collapse/expand the compose-project group of the selected container       |
| `l`                  | switch logs to selected container now (skips the follow debounce)         |
| `f`                  | toggle live follow                                                        |
| `v`                  | enter visual selection in logs                                            |
| `y`                  | yank selection (or whole buffer) to clipboard                             |
| `/`                  | filter rows                                                               |
| `?`                  | help overlay                                                              |
| `q` / `Ctrl-C`       | quit                                                                      |

Every destructive action goes through a confirmation modal. Only `y` (lowercase) confirms;
any other key cancels.

### Log insights

A row of four boxes above the logs shows what the container's logs contain, computed in dox
from the lines it already streams (a native take on [Gonzo](https://github.com/control-theory/gonzo)'s
dashboard): **Top Words**, **Top Attributes** (`key=value` and JSON fields, most distinct
values first), **Log Patterns** (Drain clustering via the `drain3` crate) and **Log Counts**
(stacked severity bars, one per refresh, with the latest counts). Counts start over when you
switch containers. The row shows when the log area is at least 100 columns wide.

Press `Enter` on a container to focus its logs; these keys then apply (dox's container
actions are ignored there, so `s`/`r` mean search/reset):

| key                 | action                                                              |
|---------------------|---------------------------------------------------------------------|
| `Tab` / `Shift-Tab` | move between logs, words, attributes, patterns, counts              |
| `↑` `↓` `k` `j`     | move the cursor (in the logs this pauses follow; `End` resumes)     |
| `Enter`             | log line details · highlight a word · a field's values · all patterns · counts analysis (60-minute heatmap, top patterns and services per severity) |
| `/`                 | regex filter on the log lines (live; `Enter` keeps, `Esc` clears)   |
| `s`                 | search and highlight text in the logs                               |
| `Ctrl+f`            | severity filter (toggle levels with `Space`, apply with `Enter`)    |
| `C`                 | columns: time, level, service, host, discovered fields              |
| `f`                 | full-screen log viewer                                              |
| `i`                 | statistics (totals, rate, severity split, pattern compression)      |
| `u` / `U`           | slower / faster refresh (0.5s to 1m)                                |
| `Space`             | pause the insights (logs keep buffering)                            |
| `r`                 | reset the insights                                                  |
| `←` `→` `h` `l`     | scroll log lines sideways                                           |
| `y` / `Ctrl+y`      | in details: copy the message / the whole entry                      |
| `Esc`               | close a popup, or leave the logs                                    |

## Config

Optional. Create `~/.config/dox/config.toml`:

```toml
refresh_ms          = 1000
log_buffer_lines    = 10000
log_tail_initial    = 500
host                = "auto"   # or "unix:///path/to/sock"
mouse               = true
confirm_destructive = true
```

## Logs

`dox` writes its own logs to your platform cache dir:
- macOS: `~/Library/Caches/dox/dox.log`
- Linux: `~/.cache/dox/dox.log`

Set `DOX_LOG=debug` to bump verbosity.

## Develop

```sh
make check    # fmt --check, clippy -D warnings, test
make test     # tests only
make run      # cargo run (debug)
```

Integration tests against a live daemon are gated:

```sh
cargo test -- --ignored
```

## What's out of scope (intentional)

- Container CPU / memory graphs
- `docker exec` shell-into-container
- Image build / pull / tag (you have `docker build` for that)
