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
rtn ask --session S "in 2 hours"        # a follow-up; same S, same conversation

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

**It is a conversation for as long as the overlay is open.** A follow-up can
refer to what was just said, so the second line here needs no repetition:

> *I need to pick up the car from the mechanic*
> Created "Pick up the car from the mechanic", unplanned.
> *in 2 hours*
> Blocked 14:00–14:30 for it today.

The transcript is `rtn`'s, kept under `$XDG_RUNTIME_DIR` — private, wiped at
logout — and forgotten when the overlay closes or after an hour idle. The
overlay only ever holds the words it draws.

### Which model

`rtn ask` makes one direct call to a model you name, on either the Anthropic
or the OpenAI wire shape. Between them those two cover the vendors and
everything that imitates one — a gateway, a local runner, another vendor's
endpoint. Say which in `~/.config/rtn/ask.yaml`:

```yaml
provider: anthropic            # or openai — and anything speaking either shape
model: claude-haiku-4-5
# base_url: https://api.anthropic.com   # the provider's own, unless a gateway
# key: env:ANTHROPIC_API_KEY            # or the key itself, with the file at 0600
```

`key:` says where the key comes from rather than leaving `rtn` to guess.
`env:NAME` reads that variable when the call is made; a bare value is the key
itself, and the file then has to be `0600` or it is refused; no `key:` at all
falls back to the variable the vendor's own tools read. So a model behind the
OpenAI shape at another host is three lines:

```yaml
provider: openai
model: grok-4
base_url: https://api.x.ai/v1
key: env:XAI_API_KEY
```

`rtn doctor` reports what asking would do — provider, model, endpoint, and
where the key would come from — and never the key. `--model` and the plugin's
`askModel` setting override the model for one call.

It can read anything, create or amend a task, and put one on the calendar; it
cannot delete, restructure, or message anyone. The tool list is an allowlist enforced twice — in what the
model is offered, and again when it calls — and that is the boundary. It runs at
most four turns and one minute. Today is put into its prompt before it starts,
so questions it already covers need no tool call at all:

> *how much free time before my next meeting?*
> Today's only event is "Catch up" at 16:00. It is 10:55 now, so you have about
> 5 hours free before it. — *one turn, no tool call*

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
inherit `~/.local/bin`, which is not guaranteed), `refreshSeconds`,
`urgentMinutes`, `askAgent` and `askModel`.

### Removing it

The plugin writes nothing outside its own directory, so taking it back is
taking back what you added:

```bash
omarchy plugin disable riclib.routine        # out of the bar, files kept
omarchy plugin remove riclib.routine         # or delete it outright
rm -f ~/.local/bin/rtn                       # the CLI, if you want that gone too
omarchy restart shell
```

Then drop the `riclib.routine` entry from `bar.layout` in
`~/.config/omarchy/shell.json` and the keybinding, if you added one. Nothing
else was touched: no menu entry, no managed block in your Hyprland config, no
file under `~/.config` that this plugin created. Your Routine data is
untouched — the journal entries and tasks it made are yours and stay where they
are.

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

## Surface, for the security review

Everything this plugin renders or acts on comes from outside the shell process,
so the boundary is where it is bounded — once, on the way in, rather than at
each place that draws it.

**What crosses into the shell.** One thing: the stdout of `rtn`, spawned as an
argument vector and never through a shell. `plugin/Model.js` is the only place
that turns it into state, and it:

- refuses a reply over **512 kB** before parsing it, because Quickshell's
  `StdioCollector` has no size limit of its own and this is the only place the
  size of what arrived can be questioned;
- refuses JSON nested deeper than **12** before `JSON.parse` sees it, walking
  the text with strings and escapes honoured so a bracket inside a string is
  content rather than structure;
- caps the lists (**20** events, **40** tasks) and every string — titles to 160
  characters, a model's answer to 4000, an error line to 300;
- takes a join URL **only** if it is `https://` with no whitespace, quotes or
  angle brackets and under 500 characters — validated whole and never
  truncated, since a shortened URL is a *different* URL to hand `xdg-open`;
- takes a task id **only** if it matches `^(task|object):[A-Za-z0-9_:.~@+-]{1,220}$`,
  because ids are echoed back to `rtn` as arguments.

The clamps live at the boundary rather than at the sinks on purpose: an answer
has four callers between the two QML files, and a fifth added later would
inherit the guard rather than the gap.

**What leaves the shell.** Only argument vectors — `rtn`, `xdg-open`,
`routine-focus`. No string is ever assembled into a command line, so quoting is
not load-bearing anywhere. `xdg-open` receives only a URL that passed the check
above, which `rtn` had already matched against an allowlist of meeting hosts.

**Rich text.** Every `Text` sets `textFormat: Text.PlainText`. Nothing here is
left on Qt's `AutoText`, which guesses rich text, and rich text loads resources
chosen by whoever wrote the string.

**Credentials.** The plugin holds none and never sees one. `rtn` reads
Routine's token from `~/.config/Routine/mcp-auth.json` at call time; the token
does not reach the shell's process state, its environment, or any file this
plugin writes.

**Subprocesses.** Every one is short-lived and started per action. Nothing is
held open, so there is no producer to orphan; a second toggle while one is in
flight queues rather than racing, and `--json` output is read to completion or
discarded.

```bash
node --test tests/          # the bounds and the shape checks, no network, no shell
omarchy plugin validate plugin/
```

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
