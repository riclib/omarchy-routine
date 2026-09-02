# omarchy-routine

[Routine](https://routine.co) on the Linux desktop: a command line, and an
[Omarchy](https://omarchy.org) shell plugin built on top of it.

Both talk to Routine through the MCP server the app ships — nothing is scraped,
nothing is reverse-engineered, and no credential leaves the machine.

```
  Wednesday 17:38  󰃰 21m          ← the bar, counting down to the next meeting
```

Press the key and the same data is a dashboard: a ring counting down, the next
event with a join button, today's tasks with boxes that tick, and a line to type
into.

---

## Why it exists

Routine's own dashboard has exactly the right shape. On this machine it takes
anywhere between no time and a minute to appear, and sometimes never.

That turned out not to be a data problem. Measured against the local MCP server,
a week of events comes back in **5 ms**, today's tasks in **1 ms**, and the
entire journal table with every note body in **1.5 ms**. Routine is local-first —
the workspace is hydrated in memory in the main process, and the MCP server runs
inside that same process, so a read never touches the network. The dashboard is
slow because it is an Electron renderer being built, not because the data is far
away.

So this is not a workaround for a slow backend. It is a different renderer over
a source that was already fast.

---

## `rtn` — the command line

One binary, for people and for programs. Everything else goes through it.

```bash
rtn log "just had a ball"              # a note in today's journal
rtn log --task pick up the milk        # a task, bound to today's entry
cat notes.md | rtn log                 # markdown, parsed into blocks

rtn today                              # the day: events, and the tasks in play
rtn next                               # the next event, and how long there is
rtn dashboard --json                   # everything the widget needs, one call
rtn task done task:…                   # tick one off  (also: open, drop)

rtn doctor                             # will any of this work, and if not why
rtn auth                               # the token, and who holds a stale copy
```

### `rtn log`

Input arrives as an argument or on stdin and is read as **markdown**, becoming
the blocks it looks like: headings, bullets, quotes, checkboxes, rules. A fenced
code block stays **one** block however long it is, which is most of the reason
the parser exists.

```bash
rtn log --date=2026-08-09 --historical < old.md
```

`--historical` makes checkboxes inert `check` blocks rather than live tasks —
for backfilling old days, whose boxes are mostly long since settled.

### Output formats

| | |
| --- | --- |
| `--pretty` | rendered for a terminal — the default when stdout is one |
| `--txt` | plain prose, markers stripped — the default when it is not |
| `--md` | markdown as written |
| `--json` | fields, for programs |

Every human-facing command builds markdown and the first three are renderings of
that one string, so output composes:

```bash
rtn --md today | rtn log      # the day, logged back into the journal as blocks
```

### There is no login

Routine's token file **is** the credential, and `rtn` re-reads it on every call,
so it has nothing of its own to expire. `rtn auth` exists for the clients that
*do* keep a copy — Claude Code stores a literal `Authorization` header, and
anything that regenerates the token leaves it 401ing in a way that does not look
like an auth problem.

```bash
rtn auth --mcp-config > ~/.config/routine-mcp.json && chmod 600 $_
```

writes a config another MCP client can load, without the token passing through a
terminal or a chat.

---

## The Omarchy plugin

Two surfaces, one data source.

**A bar widget** showing how long until the next thing — permanently, which is
the one thing a dashboard you have to open cannot do. It goes urgent inside five
minutes. Left click focuses Routine, middle click joins the next meeting, right
click refreshes.

**An overlay** with the dashboard: the countdown ring, the next event with its
platform and a Join button, today's list with working checkboxes, and a capture
line. `Enter` logs a note, `Shift+Enter` logs a task, `Esc` closes.

Ticking a box completes the task and the note's checkbox together, because in
Routine they are two views of one thing.

### Installing it

```bash
cargo build --release && install -Dm755 target/release/rtn ~/.local/bin/rtn
cp -a plugin ~/.config/omarchy/plugins/riclib.routine
omarchy restart shell
```

Or `./bin/install`, which does all of that and then tells you what landed.

Add `riclib.routine` to a section of `~/.config/omarchy/shell.json`, and bind the
overlay to a key:

```lua
o.bind("MOD3 + R", "Routine dashboard", "omarchy-shell shell toggle riclib.routine '{}'")
```

**Settings:** `rtnBin` (leave it as `rtn` unless the graphical session does not
inherit `~/.local/bin`, which is not guaranteed), `refreshSeconds`, and
`urgentMinutes`.

---

## How it is put together

**The QML never speaks MCP.** `rtn` holds the token, bounds every string, and
decides which links are safe to open; the widget only paints what comes back. A
credential does not belong in a shared shell process, and neither does the job
of parsing a meeting invite.

That boundary does real work. An event description is a hostile thing — a Teams
invite is thousands of characters of dial-in numbers, legal text and
angle-bracketed links, with one useful item buried in it. `rtn` extracts the
join link against an **allowlist of hosts**, matching on the host rather than
the URL so that `example.com/zoom.us/j/1` and `teams.microsoft.com.evil.test`
are both nothing, and the description itself never leaves the process.

**The countdown is local arithmetic** on one fetched timestamp. The data costs a
call a minute; the clock face costs nothing.

**Nothing waits for a round trip it does not have to.** Ticking a box fills it
immediately — the task changes at once, but the note's checkbox trails it by
about five seconds through the app's own sync, and a box that waits for the
truth looks broken.

## Requirements

Routine, with its MCP server enabled in **Settings → MCP** (it is off by
default). Rust to build. The plugin additionally wants Omarchy's Quickshell
`omarchy-shell`; `rtn` alone needs neither.

## Notes on Routine's MCP surface

`CLAUDE.md` is the working record: what the server does, measured rather than
assumed, including a handful of behaviours that are not documented anywhere and
that bit hard enough to be worth writing down. Some of it may be useful to
anyone else building against it.

## License

MIT.
