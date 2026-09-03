# omarchy-routine — working notes

An Omarchy shell plugin fronting [Routine](https://routine.co): the dashboard as
a Quickshell overlay, a countdown to the next meeting in the bar, and capture
straight into today's journal.

**Why it exists.** Routine's own dashboard (`routine://dashboard`) has the right
shape — a ring counting down to the next event, a NEXT card, today's tasks, a
quick-create box — and everything it renders is reachable over the MCP server.
Drawing it in the shell instead makes it a keystroke rather than a window, and
puts the countdown on the bar permanently, where a window cannot be.

**What exists:** `rtn`, the Rust CLI everything else goes through, and the
plugin — a bar countdown and the dashboard overlay. What is *not* built is the
bar panel, which would be a smaller version of the overlay.

## Gotchas, and how to read them

Everything below was measured against a live server on 2026-09-02 (Routine MCP
server 2.2.0, 52 tools) on Arch Linux under Hyprland.

**Treat all of it as observed behaviour with a date on it, not as contract.**
The MCP server is new, Linux is a newer place for Routine than macOS, and a
desktop shell asking these questions sixty times an hour is not the use it was
first shaped around. Several of the notes here are the ordinary friction of
being early to a surface — the sort of thing that gets smoothed as more people
build on it, and some of which may already be different by the time you read
this. Re-verify after a Routine update rather than trusting the page.

They are written down because each one cost real time to find, and none of them
is guessable from the schema. If you are building the second one of these, start
here.

## The transport

`127.0.0.1:8765/mcp`, bearer auth, and — the useful part — **stateless**. No
`initialize` handshake, no session id, no `notifications/initialized`. One POST
carrying a `tools/call` gets a result:

```bash
curl -sS -X POST http://127.0.0.1:8765/mcp \
  -H "Authorization: Bearer $(jq -r .value ~/.config/Routine/mcp-auth.json)" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
       "params":{"name":"tasks_listTodaysTasks",
                 "arguments":{"workspace":"workspace:…","limit":20}}}'
```

- Replies are SSE-framed — `event: message` / `data: {…}`. One `sed -n 's/^data:
  //p'` gets you JSON. There is no streaming; it is a single frame.
- **The token lives in `~/.config/Routine/mcp-auth.json`** (0600, plaintext —
  see `bin/routine-focus` in omarchy-cust for why plaintext is the deliberate
  choice). Read it at call time. Never copy it into a config the way Claude
  Code does, or it goes stale the moment Routine rewrites the file.
- The MCP server is **off by default** — Settings → MCP in the app.
- The port only exists while Routine is running, so "app is closed" arrives as
  a connection refusal, not a hang. Bad token is a clean 401.

### It is a memory read, not a network call

Measured, five reps each: a week of events **5 ms**, today's tasks **1 ms**, the
entire journal table with every note body **1.5 ms** (28 kB), all 121 tasks
**3 ms**. Nothing here touches the network.

Routine is local-first. The workspace lives in one file —
`~/.local/share/Routine/workspaces/<workspace-id>`, 7.9 MB on this machine — which is a
bespoke length-prefixed binary op log (`01 00 00 00…` then a length-prefixed
nanoid; not SQLite, not LevelDB, not Automerge) carrying operations from 166
device actors. **There is no IndexedDB**: nothing mirrors that log into the
renderer, and Local Storage holds 20 kB of Segment ids and spellcheck settings.
State is hydrated in memory in the main process, and the MCP server runs inside
that same process — which is why it answers in single-digit milliseconds.

**Do not read the op log directly.** No schema, no index, no documentation,
rewritten live, and it would break on any Routine update — the MCP server is the
supported way in and it is fast enough that there is no reason to want another.
Worth knowing that it sits at `0644` under `0755` directories, so on a shared
machine it is readable by other local users; not a reason to add a second
reader.

**What this says about the plugin.** The data layer is not the constraint — it
answers in milliseconds from the same process that draws the window. So a native
shell surface is not compensating for anything slow; it is a lighter renderer
over a source that was already fast, and that is the whole reason this is worth
building.

## Response shapes differ between tools

Most tools return `result.structuredContent` already parsed. **Two do not** and
return only a text block holding JSON:

- `search_search`
- `tables_searchTableRows`

Handle both shapes or those two come back empty. Parse `structuredContent` when
present, else `json.loads(result.content[0].text)`.

**`tables_listTables` and the app's type picker can disagree.** The picker shows
fifteen object types; `listTables` returned fourteen, without `projects`. The
table is entirely usable — `getTableSchema` and
`searchTableRows` both answer on it — it is just absent from the enumeration.
Find it the way anything else undocumented is found:

```json
search_search  {"query": "project", "kind": {"type": "table"}}
→ table_ref:<workspace>:<nanoid>  "projects (Project)"
```

Note the id comes back as `table_ref:<workspace>:<nanoid>`; the table tools want
`table:<nanoid>`. The practical rule: an entity missing from the enumeration is
not necessarily out of reach — look for it before concluding it is.

## What the data actually looks like

**Events come back unsorted.** `listEventsForDateRange` returned Sep 2, Sep 5,
Sep 4, Sep 2, Sep 4 for a three-day range. Sort client-side or "next meeting"
lies.

**Event descriptions arrive as the sender wrote them.** A Teams invite is
thousands of characters of dial-in numbers, `<…>`-wrapped links and corporate
legal text, with the one useful item — the join URL — buried in it. Routine
passes it through faithfully, which is right; the trimming is the client's job. Extract the join link against an allowlist
of meeting hosts in the helper; never let the description itself reach QML.
`location` is the cheap signal (`"Microsoft Teams Meeting"`), and matching it
against a small bounded pattern set is how the platform glyph gets chosen.

**Titles contain newlines.** Real examples from `tasks_listTasks`: `"Send
proposal to Untitled\n "`, `"\nAutomate solidmon components upgrade\n"`. Collapse
whitespace before anything renders.

**`tasks_listTasks` is id + title only.** 121 tasks, `truncated: true`, ordered
by id, no dates and no completion state. Detail is one `getTask` each. Treat the
list tools as an index, never as a payload.

**`scheduled` is a bare string**: `"2026-09-02"` for a day, `YYYY-WW` for a week
batch ("Week 36" in the app). It is absent, not null, when unset.

**Scheduling is currently one-way over MCP.** `updateTask` with
`scheduled: null` answers success and changes nothing — `null` is the schema's
"leave alone" default, and there is no separate sentinel for empty:
`{"date":""}` and `{"week":""}` fail to parse, `"unplanned"` and `{}` fail to
match `scheduled_input`. The app can unplan a task; a client cannot yet. Until
there is a way to say it, do not build an affordance that needs one.

**`tasks_allocateTask` is the only way to a time of day, and it schedules as a
side effect.** Measured 2026-09-03: `{task, date, start_time: "14:00",
duration_minutes: 30}` answers `{"eventId": "event:…"}`, the task's
`allocationIds` gains that id, and `scheduled` is set to the date whether or
not it was before. Deleting the event with `personal_events_deleteEvent` empties
`allocationIds` and leaves `scheduled` where it is — and scheduling being
one-way, nothing over MCP can put it back. An allocated task is on the day for
good, from a client's point of view.

**An allocated task vanishes from `listTodaysTasks`** — neither `todo` nor
`other` — and appears instead as an ordinary event in
`listEventsForDateRange`, with the task's title and nothing in the list entry to
mark it. Only `getEvent` shows `allocationOfTask: "task:…"`, and the task's
own done flag is `completed` on `getTask` (`status` is always null there). So
the dashboard reads the day as an **agenda**: every event gets its `getEvent`
(a millisecond each), a block is one with `allocationOfTask`, and its task is
looked up for `completed` and kept out of the Anytime list. The ring counts to
the next agenda item of either kind, by decision: a block is committed time.

Corollary for capture: **create tasks unplanned and leave them there.** A task
created with a journal `parent` and no `scheduled` comes back with no
`scheduled` key, and the app still shows it under Unplanned with a chip naming
the day it was captured — the parent already carries that. Planning it as well
puts it in Wednesday for no gain, and cannot be undone from here.

## Today is a union of two reads

The most surprising shape here, and the one worth knowing before building the
obvious thing.

A task whose only anchor is a **parent pointing at today's journal row** — which
is what every checkbox typed into the daily note is — appears in **neither**
`listTodaysTasks` nor `listUnplannedTasks`. The app shows it in both Today and
Unplanned. Only `scheduled` membership and completed-today tasks come back from
`listTodaysTasks` (completed ones land in its `other` group).

So the Today card is a union of two reads:

1. the `todo` blocks of today's journal row — these carry `checked` and the
   bound `task:` id, and *are* what the daily note renders
2. `listTodaysTasks.todo` — for tasks scheduled today that are not in the note

Both are one call. Neither is sufficient alone.

## The journal, and how to append without disturbing it

Routine's daily note is a row in a user-defined table (`journal`, here
`table:daily_notes__…`) with two columns, `Title` and `Notes`. The
`Notes` document is a list of typed blocks — and the todo blocks in it are
bound to real tasks. **The daily note is the task list.**

Three facts govern every write to it:

**There is no append yet.** `tables_write_updateNotesColumn` replaces the entire
document. The `/daily` house rule — *the day belongs to other tools too; append
at the end and touch nothing above it* — therefore has to be constructed.

**`{"type":"existing","id":"block:…"}` is how you construct it.** It is the
tenth writable block type and it takes an id and nothing else. Echo every block
you are not adding by reference, append yours, write the lot:

```json
{"blocks":[
  {"type":"existing","id":"block:…"},
  {"type":"existing","id":"block:…"},
  {"type":"paragraph","content":"the new line"}
]}
```

Nothing is re-serialised, so a block type the write schema cannot model — an
image, a table, whatever ships next — survives untouched. It also means the read
only needs the **ordered id list**; discard the rest rather than caching a whole
day's note.

The nine block types you can actually author: `paragraph`, `heading` (content,
level, retracted), `bullet` (list_type, depth), `check` (checked, depth), `todo`
(checked, content, task), `blockquote`, `code` (language), `divider`, `embed`.

**Blocks exist that are not on that list, and they are not hypothetical.** A
project note here holds a `query` block — Routine has a query language embedded
in documents:

```
tasks where 'Project'._routine.id = "row:…"
index by 'Status' as _routine_source_database
select (…)
```

`query` cannot be authored. A document rebuilt from parsed content would have
destroyed it silently. This is what `existing` is for, and why the rule is echo
by reference rather than round-trip the content.

**Markdown is the storage format.** Block content round-trips as literal
characters over the API — `**bold**` comes back as `**bold**` — but the app
parses it at display time. Verified by eye on an imported note: bold renders,
`[text](url)` becomes a link, backticks become code chips, `mailto:` resolves.
So author markdown in `content` and do not try to pre-render it. The `/daily`
house style — bold the claim, backtick anything typed, link the chat — ports
over unchanged.

The corollary is a security one: a block's content is markdown, and it arrives
from the server. Anything rendering a note in QML is rendering untrusted
markup, which is exactly what `textFormat: Text.PlainText` exists for.

**A day with no entry has no row.** Searching the journal table for a date with
nothing in it returns `[]`. The first capture of the morning has to create the
row, and `addTableRow` explicitly refuses `Note` columns — so it is create-row,
then `updateNotesColumn`. Row lookup is by `Title` matching a locale-formatted
string like `"September 2, 2026"`, which is a fragile key; treat a miss as
"needs creating" only after ruling out a formatting mismatch.

Three things about that branch, each of which breaks it if missed. Verified by
creating tomorrow's day ahead of time rather than finding out at 8am:

- **A field value wants the typed envelope**, the same one reads come back in:
  `{"name":"Title","value":{"type":"string","value":"September 3, 2026"}}`. A
  bare string is rejected — *expected object, got "September 3, 2026"*. The
  `schema: read_only` on the journal's columns is about altering the schema, not
  the values; Title writes fine.
- **`addTableRow` answers `{"id":"object:<ws>:<table>:<row>"}`** — the full
  compound id, so there is nothing to reassemble.
- **A brand-new row's `Notes` is `{"type":"null"}`**, not an empty document.
  There is no `blocks` key to read, so anything walking the note has to treat an
  unset Notes as an empty list. This is the one that would have thrown on the
  first capture of every morning.

Once written, the row is found by the ordinary Title lookup, so the second
capture of the morning reuses it rather than making a second day.

And the row is a real journal day, not an orphan: a day created this way ahead
of time opens in the app with its normal day header and the written content in
place. Worth having checked — Capacities fails exactly here, where only
`saveToDailyNote` can make the day and the REST append cannot, so a capture
aimed at a day that does not exist yet has to queue. Routine needs no such
outbox: **the plugin can always write, whatever day it is.**

**Append before the trailing empty paragraph.** Routine keeps an empty paragraph
last. Splice in front of it rather than after, or content lands below a blank
line.

### A checkbox takes two calls, in this order

**Give a `todo` block a real task id rather than a null one.** Measured
2026-09-02, twice in one write. The MCP mints a task for the block server-side; the Electron client,
syncing the same document, sees a todo block and mints its own. **Two tasks per
block**, and which of them wins the binding is not predictable — in the same
import, one pair bound the note's copy and the other bound the orphan. Both are
parented to the journal row, so both appear in the app's Unplanned list while
neither comes back over MCP. It is worth avoiding rather than detecting: the
calendar block dragged out of the orphan is not the task the checkbox completes,
and nothing announces that.

**And `createTask(parent = <journal row>)` does not insert a block.** The
relationship runs one way — a todo block mints a task; a parented task does not
mint a block. Three tasks created against today's row left the note untouched.
So the block has to be authored; there is no atomic path that does both.

The order that works, verified clean (exactly one task per checkbox, no
double-mint):

1. `tasks_createTask(workspace, title, parent={kind:"object", id:<row>})`
   → answers `{"taskId": "task:…"}`, **not** `id`
2. author the block with that id: `{"type":"todo", "checked":false,
   "content":…, "task":"task:…"}`

An explicit task id is what stops the client minting a second one. Make the
import idempotent by reusing an open task already under today's row with the
same title, or a re-run duplicates every checkbox.

`tasks_deleteTask` answers `null` on success.

### The task and its checkbox converge in about five seconds

Traced both ways. `updateTask` changes the task at once and the note's `checked`
follows roughly five seconds later, through the app's own sync — and it works in
both directions with no bounce-back:

```
after done  +1s  task=True  block=False      after open  +1s  task=False block=True
after done  +6s  task=True  block=True       after open  +6s  task=False block=False
```

Two consequences for anything with a checkbox in it. **A dashboard reads a
journal task's done state from the block**, so it lags the task — tick
optimistically or the row looks broken for five seconds. And **do not clear
optimistic state wholesale on a refresh**: a poll landing inside that window
reports the old value and flips the box back while the user watches. Drop only
the ticks the server has caught up with.

### Rewriting the note does not disturb its tasks

Verified by allocating a task, rewriting the whole document with every block by
`existing` reference, and reading back: `allocationIds` and `scheduled` both
survived, block count unchanged. So reordering and appending are safe against
tasks that are scheduled or blocked out on the calendar — which is what makes
"create the tasks, then place them by reference" a usable pattern rather than a
destructive one.

## Concurrency

The ordered block-id list is the concurrency token. There is no ETag.

- **For an append: re-read the ids immediately before writing and use the fresh
  list.** Appending commutes with anything happening above it, so there is
  nothing to abort over, and the loss window shrinks to one local round trip
  (~80ms). Only a simultaneous append can collide.
- **Abort on mismatch only when order matters** — chiefly the Ctrl-P plan path,
  where the plan is ~13s stale by the time it applies. Verified working: after
  the note was edited by hand, a write built on the pre-edit id list correctly
  refused rather than resurrecting deleted blocks.

## The ask path, and what it costs

Quick-create splits by how the input was submitted, not by guessing at it:

| Key | Behaviour |
| --- | --- |
| `Enter` | append to today's journal — deterministic, no model |
| `Shift+Enter` | file a task in the Inbox, unplanned — deterministic, no model |
| `Tab` | switch the same box to asking, and back |

Which mode you are in is a decision you make, not one inferred from what you
typed — cue-detection is a guess, and a capture box that sometimes costs five
seconds and a model call is a capture box you stop trusting. Enter and
Shift+Enter never do.

`rtn ask` makes one direct HTTPS call to the model named in
`~/.config/rtn/ask.yaml`, on the Anthropic or the OpenAI wire shape through
[`llm-wires`](https://github.com/riclib/llm-wires), with the key held as a
`wire_secret::Secret` from the moment it is parsed to the header it lands in.
It fetches Routine's own tool list over MCP (`tools/list`), filters it to
`ALLOWED`, and hands the schemas over untouched — they are already on the wire,
and hand-copying them is how they go stale. The loop is bounded three ways:
four turns, 16 kB per tool result, sixty seconds wall clock.

**A `--session` makes it a conversation.** The overlay mints an id when it
opens and says `--end` when it closes; `rtn` keeps the transcript under
`$XDG_RUNTIME_DIR/rtn/` at 0600, forgets it after an hour idle, and trims it by
whole exchanges — a tool call and its result must stay paired or the API refuses
the request. The follow-up needs no new intelligence: the `createTask` result in
turn one carries the task id that "in 2 hours" in turn two needs.

**The system prompt carries no clock.** It is the cached prefix, and `It is now
13:10` in it changed every minute, so across questions the cache never hit — in
a conversation that is the whole prefix, every turn. The time is prefixed to
the user message instead; the TODAY block stays in the system prompt and is
refreshed per question, so it changes exactly when the day did, which after a
write is the one time re-reading it is worth paying for. On the Anthropic wire
tools come before system in the prefix, so a changed TODAY still leaves the
twenty schemas cached.

**The allowed tool list is the security boundary, and it is enforced twice**:
the model is only offered allowed tools, and every call is checked again at the
point of execution, because a model can name a tool it was never given. A
refused call comes back to the model as a tool result saying so, not as a dead
question. Reads are broad; writes are `createTask`, `updateTask` and
`allocateTask`; there is no delete, no `tables_alter_*`, no other workspace and no `notices_createNotice`.
Verified 2026-09-03 with a scripted model: a `tasks_deleteTask` it asked for came
back as `rtn does not allow tasks_deleteTask` and nothing was deleted.

Until 2026-09-03 this ran the `claude` CLI headlessly with `--allowedTools`.
Three things retired it: the harness's startup was most of the latency; Omarchy
standardises *which* coding agent but there is no headless contract across the
ten, and MCP wiring differs per agent, so the path refused to run for anyone
whose default was not `claude`; and the allowlist was a flag passed to someone
else's process rather than a match arm in this one. The measurements below were
taken through that harness and are kept because they are the reason for the
shape: the priming is what makes it one turn, and one turn is what makes it
usable.

The thing that makes it usable is **priming the prompt**. Today's dashboard goes
in before the agent starts, which costs 10 ms locally and removes the model
round trips that are the entire latency. Measured on the same machine:

| | |
| --- | --- |
| a question today's context answers | **1 turn, 5.8s, $0.036** |
| a question needing a tool, or a write | 6 turns, ~19s |
| the same question with no primed context | 6 turns, 50s |

The system prompt carries the field notes below as a briefing — events being
unsorted, Today being a union, scheduling being one-way, create-unplanned. An
agent that does not know them makes exactly the mistakes this file records.

An earlier design had the model propose a plan and a validator execute it.
Measured on one representative input:

| Config | Wall | Turns | Cost | Cache read |
| --- | --- | --- | --- | --- |
| Fable 5.1 + 52 MCP tools | 39s | 11 | $0.71 | 187k |
| Haiku 4.5 + 52 MCP tools | 37s | 6 | $0.056 | 113k |
| **Haiku, no tools, pre-fetched context** | **13.7s** | **1** | **$0.019** | **17.8k** |

Latency is the harness, not inference — Haiku with tools was no faster than
Fable. The win comes from deleting the tool round trips: the helper does in
~80ms of local HTTP what the agent spent six model turns on. The remaining ~13s
was `claude -p` startup — which is what the direct call removes, at the price of
needing a key. The tool loop's own overhead, measured against a local fake
provider with live Routine behind it, is 6 ms for three turns.

Two things the no-tools model got wrong, both handled by the validator rather
than by prompting harder: it flattened `time` into top-level
`start_time`/`end_time` (no tool schema to copy), and it fenced its JSON in
` ```json `. Model output is another untrusted string arriving at the shell.

Giving it the read tools did buy real judgement — asked to schedule a call with
someone by first name only, it picked the right one of four contacts sharing
that name by reading which project the call was about. Haiku without tools did
not. If that is wanted back, the helper should do the contact search and put the
candidates into the context rather than handing over the tool.

## House rules

Inherited from `riclib.capacities`, which earned them under marketplace review.

- **The QML never talks to the API.** Token, bounding, caching and queueing live
  in `bin/`. A credential does not belong in the shell's process state.
- **Refresh on a user action; a slow tick is affordable.** Capacities forbids
  polling outright because its quotas are 10 requests per 60s. That rationale
  does not transfer — Routine's reads are 1–5 ms against local memory with no
  quota at all, so a ring that re-reads every half minute costs nothing worth
  counting. Keep the ticking itself local arithmetic on one fetched timestamp,
  and poll slowly because waking the shell has a cost even when the call does
  not — not because the API will object.
- **Bound at the boundary, not at the sink.** Clamp on the way out of the helper
  so a stale cache is bounded on read too.
- **A capture must never be lost.** A plan that fails validation degrades to a
  plain task carrying the raw text. The model may be smart; it may never be the
  reason something vanished.
- **Every `Text` sets `textFormat: Text.PlainText`.** Qt defaults to `AutoText`,
  which guesses rich text, and rich text loads resources. The marketplace
  security review flags exactly this.
- **Don't tidy.** The day belongs to other tools. Append at the end and touch
  nothing above it.

## Multi-monitor

The dashboard wants to be an `overlay`, not a bar panel — which sidesteps the
bug `nixfred.blip` has, where the panel is anchored to the leader instance's bar
button and so always opens on one screen regardless of where you clicked. An
overlay is not anchored to a button at all. See `omarchy/PLUGINS.md` in
omarchy-cust for the full account of that failure.

For the bar widget: IPC routes to one handler, but bar widgets are live once per
monitor. Resolve the focused instance via `bar.findPanelWidget` rather than
letting whichever registered first answer. Refreshes broadcast, because a
refresh is not a place.

## The shell's styling API, which is not guessable

Invented names fail at runtime, not at load, so they show up as a widget that
draws wrong rather than one that refuses. The real surface:

| | |
| --- | --- |
| `Style.space(px)` | a pixel value times the theme's spacing scale — used with real numbers, e.g. `Style.space(560)` for a card |
| `Style.spacing.{hairline,xxs,xs,sm,md,lg,xl}` | named gaps, for anything between elements |
| `Style.font.{caption,bodySmall,body,subtitle,title,display}` | type sizes. There is **no** `Style.fontSize(n)` |
| `Style.cornerRadius`, `Style.gapsOut` | mirror Hyprland's rounding and half its `gaps_out` |
| `Color.urgent` | lives at the root, not under `Color.menu` |
| `Color.menu.*` | the launcher's surface tokens — share them and a theme styles this too |

`qs.Ui` exports its own `TextField`, so importing `QtQuick.Controls` alongside
`qs.Ui` is both ambiguous and the wrong choice: the shell's field is themed.

## Gotchas that cost an afternoon elsewhere

Carried over from `riclib.capacities` — all of these apply here too.

- **Components are cached by URL, and it is worse than the Capacities note
  says.** Editing `BarSlot.qml` in place does nothing without `omarchy restart
  shell` — but neither does a **newly added overlay**, nor any sub-component it
  loads (`Ring.qml`, `TaskRow.qml`). Overlays hot-reload only once the shell has
  successfully loaded them at least once.

  A stale cache announces itself as **`File name case mismatch` at
  `[-1:-1]`**, which reads like a filename problem and is not one. An hour went
  into renaming the file, removing siblings and diffing manifests before
  `omarchy restart shell` fixed it untouched. The same cache also reports errors
  against lines that no longer exist — `Style.fontSize is not a function` kept
  firing from a file with no such call in it. **When a QML error stops matching
  the file you are reading, restart the shell before believing it.**
- **`data` is `Item`'s default property.** `property var data` silently shadows
  the child-object list. Name it anything else.
- **Naming a QML file after the type it extends breaks it** — the local
  directory is an implicit import, so the type resolves to itself.

## Related

- `~/src/omarchy-cust` — `omarchy/PLUGINS.md` (plugin provenance on this desk),
  `bin/routine-focus` (why no `--password-store` flag), `hypr/bindings.lua`
  (`SUPER+R`, `MOD3+R`).
- `~/src/omarchy-capacities` — the model for a plugin with a working remote, and
  the source of the house rules above.
