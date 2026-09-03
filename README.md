# omarchy-routine

[Routine](https://routine.co) on the Linux desktop: a command line, and an
[Omarchy](https://omarchy.org) shell plugin built on top of it.

https://github.com/user-attachments/assets/b1c56689-b43f-43f7-8ab5-fcf1a2c761f0

and

https://github.com/user-attachments/assets/acdaa5f6-b59b-4719-a9a8-20c41bccf5f9

Both are built on the MCP server Routine ships — nothing is scraped, nothing is
reverse-engineered, and no credential leaves the machine. Routine did the part
that makes this possible; this is just a second surface onto it.

```
  Wednesday 17:38  󰃰 21m          ← the bar, counting down to the next meeting
```

Press the key and the same data is a dashboard: a ring counting down, the next
event with a join button, today's tasks with boxes that tick, and a line to type
into.

---

## Why it exists

Some things want to live in the desktop rather than in a window: how long until
the next meeting, whether today is clear, a thought you want written down before
it evaporates. Not because the app is missing them — Routine has all of it — but
because a window you have to bring forward is a different thing from a number
that is simply always on the bar, and a capture box a keystroke away is a
different thing from one a context switch away.

**It works because Routine is local-first.** The workspace is hydrated in memory
in the app's main process, and the MCP server runs inside that same process, so
a read never touches the network. Measured here: a week of events in **5 ms**,
today's tasks in **1 ms**, the entire journal table with every note body in
**1.5 ms**. That is what makes a bar widget honest — it can ask sixty times an
hour and cost nothing, so the countdown on your bar is never stale and never a
guess.

Very little of this would be worth building against a cloud API with a rate
limit. It is worth building against a local one that answers in a millisecond.

---

## `rtn` — the command line

One binary, for people and for programs. Everything else goes through it.

```bash
rtn log "just had a ball"              # a note in today's journal
rtn log --task pick up the milk        # a task, bound to today's entry
cat notes.md | rtn log                 # markdown, parsed into blocks

rtn add call the plumber               # a task in the Inbox, unplanned
rtn ask "how much free time before my next meeting?"

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

**A bar widget** showing how long until the next thing, permanently, with no
window in the way. It goes urgent inside five minutes. Left click focuses
Routine, middle click joins the next meeting, right click refreshes.

**An overlay** with the dashboard: the countdown ring, the next event with its
platform and a Join button, today's list with working checkboxes, and one box
at the bottom that does three things.

| | |
| --- | --- |
| `Enter` | append to today's journal |
| `Shift+Enter` | file a task in the Inbox, unplanned |
| `Tab` | switch the box to asking, and back |
| `Esc` | out of asking, then out of the overlay |

**Today stays on screen while you ask**, because you are usually asking about
what you can see — glance at the list, notice the thing that is missing from it,
Tab, say so, and watch it arrive:

> *add buy potatoes to today*
> Done. "Buy potatoes" is created and scheduled for today, September 3. That
> makes 4 open tasks today.

The new row animates into the list as the sentence appears, so the confirmation
is the list changing rather than only a claim that it did.

### Which agent

Omarchy already has a convention for this — `omarchy default agent`, stored in
`~/.config/omarchy/defaults/agent` — and `rtn ask` follows it rather than
inventing its own. What Omarchy standardises is *which* agent; there is no
shared contract for driving one headlessly, and MCP wiring in particular
differs: Claude takes a config **per invocation**, so it can be handed Routine
and nothing else, while `grok`, `codex` and `opencode` read theirs from
persistent config.

That scoping is the security boundary rather than a convenience, so `rtn ask`
drives only agents where it holds — today that is `claude`. If your default is
another, it says so and names the two ways out rather than quietly running
something you did not choose. `--agent` and the plugin's `askAgent` setting
override it.

Asking runs `rtn ask`, which is the `claude` already on your PATH pointed at
Routine's MCP server and nothing else. It can read anything and create or amend
a task; it cannot delete, restructure, or message anyone — the tool list is an
allowlist and that is the boundary. Today is put into its prompt before it
starts, so questions it already covers need no tool call at all:

> *how much free time before my next meeting?*
> Today's only event is "Catch up" at 16:00. It is 10:55 now, so you have about
> 5 hours free before it. — **one turn, 5.8s**

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

That boundary does real work. A calendar invite arrives as whoever sent it wrote
it — a Teams one is thousands of characters of dial-in numbers, legal text and
angle-bracketed links, with the one useful item buried in it. `rtn` extracts the
join link against an **allowlist of hosts**, matching on the host rather than
the URL so that `example.com/zoom.us/j/1` and `teams.microsoft.com.evil.test`
are both nothing, and the description itself never leaves the process.

**The countdown is local arithmetic** on one fetched timestamp. The data costs a
call a minute; the clock face costs nothing.

**Nothing waits for a round trip it does not have to.** Ticking a box fills it
immediately. The task completes at once and the note's checkbox follows a moment
later as the app syncs the document, so the widget shows the tick straight away
and reconciles when the note catches up.

## Requirements

Routine, with its MCP server enabled in **Settings → MCP** (it is off by
default). Rust to build. The plugin additionally wants Omarchy's Quickshell
`omarchy-shell`; `rtn` alone needs neither.

## Field notes and gotchas

`CLAUDE.md` is the working record kept while building this: how the MCP server
behaves, measured rather than assumed, with the traces behind each finding. It
covers the shapes that are not obvious from the schema — how a daily note's
checkboxes relate to tasks, how to append to a note without disturbing what is
already in it, how the two sides converge after a write.

Everything in it is **observed behaviour with a date on it, not contract**. The
MCP server is new and Linux is a newer place for Routine than macOS, so some of
it is the ordinary friction of being early to a surface, and some may already
have changed. Re-verify after an update rather than trusting the page — and if
something there is now wrong, a PR fixing it is welcome.

It is written for whoever builds the next one of these.

## Thanks

To the Routine team for shipping an MCP server at all, and for making it local
and fast enough that a desktop integration is a few milliseconds rather than a
sync problem. And to [Omarchy](https://omarchy.org), whose shell makes a plugin
like this a couple of QML files rather than a project.

## License

MIT.
