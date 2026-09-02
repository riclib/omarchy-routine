# omarchy-routine — working notes

An Omarchy shell plugin fronting [Routine](https://routine.co): the dashboard as
a Quickshell overlay, a countdown to the next meeting in the bar, and capture
straight into today's journal.

**Why it exists.** Routine ships its own dashboard (`routine://dashboard`) with
the shape we want — a ring counting down to the next event, a NEXT card, today's
tasks, a quick-create box. It takes anywhere between instant and 20–60s to load,
and sometimes never. Everything it renders is reachable over Routine's MCP
server, so the widget can be a Quickshell overlay that is always there.

Nothing here is built yet. What *is* established is the data layer below, all of
it measured against a live server on 2026-09-02 (Routine MCP server 2.2.0,
`routine-mcp-server`, 52 tools). Re-verify after a Routine update; the app is
young and several of these are behaviours rather than contracts.

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

## Response shapes, which are not uniform

Most tools return `result.structuredContent` already parsed. **Two do not** and
return only a text block holding JSON:

- `search_search`
- `tables_searchTableRows`

Any client must handle both or those two silently return nothing. Parse
`structuredContent` when present, else `json.loads(result.content[0].text)`.

## What the data actually looks like

**Events come back unsorted.** `listEventsForDateRange` returned Sep 2, Sep 5,
Sep 4, Sep 2, Sep 4 for a three-day range. Sort client-side or "next meeting"
lies.

**Event descriptions are hostile.** A Teams invite is thousands of characters of
dial-in numbers, `<…>`-wrapped links and corporate legalese, with the one useful
item — the join URL — buried in it. Extract the join link against an allowlist
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

**Planning is one-way — a schedule cannot be removed over MCP.** `updateTask`
with `scheduled: null` answers success and changes nothing, because `null` is
the schema's "leave alone" default, and there is no sentinel for empty:
`{"date":""}` and `{"week":""}` fail to parse, `"unplanned"` and `{}` fail to
match `scheduled_input`. The app can unplan a task; the plugin never will be
able to. Do not design an affordance that needs it.

Corollary for capture: **create tasks unplanned and leave them there.** A task
created with a journal `parent` and no `scheduled` comes back with no
`scheduled` key, and the app still shows it under Unplanned with a chip naming
the day it was captured — the parent already carries that. Planning it as well
puts it in Wednesday for no gain, and cannot be undone from here.

## `listTodaysTasks` is not what feeds the app's Today

The single most surprising finding, and it will bite any dashboard built the
obvious way.

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

## The journal, and how to append without destroying it

Routine's daily note is a row in a user-defined table (`journal`, here
`table:daily_notes__example`) with two columns, `Title` and `Notes`. The
`Notes` document is a list of typed blocks — and the todo blocks in it are
bound to real tasks. **The daily note is the task list.**

Three facts govern every write to it:

**There is no append.** `tables_write_updateNotesColumn` replaces the entire
document. The `/daily` house rule — *the day belongs to other tools too; append
at the end and touch nothing above it* — therefore has to be constructed.

**`{"type":"existing","id":"block:…"}` is how you construct it.** It is the
tenth writable block type and it takes an id and nothing else. Echo every block
you are not adding by reference, append yours, write the lot:

```json
{"blocks":[
  {"type":"existing","id":"block:sDrArgOdgx4yQKNZszrpK"},
  {"type":"existing","id":"block:ALM-1XGR1tNxCiepn5UQp"},
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

**Markdown is not parsed.** Block content is a flat string; `**bold**` and
`[text](url)` are stored as literal characters. The `/daily` house style (bold
the claim, backtick the commands, link the chat) does not survive a port to
Routine as-is.

**A day with no entry has no row.** Searching the journal table for a date with
nothing in it returns `[]`. The first capture of the morning has to create the
row, and `addTableRow` explicitly refuses `Note` columns — so it is create-row,
then `updateNotesColumn`. Row lookup is by `Title` matching a locale-formatted
string like `"September 2, 2026"`, which is a fragile key; treat a miss as
"needs creating" only after ruling out a formatting mismatch.

**Append before the trailing empty paragraph.** Routine keeps an empty paragraph
last. Splice in front of it rather than after, or content lands below a blank
line.

### A checkbox takes two calls, in this order

**Never write a `todo` block with `task: null`.** Measured 2026-09-02, twice in
one write. The MCP mints a task for the block server-side; the Electron client,
syncing the same document, sees a todo block and mints its own. **Two tasks per
block**, one of which wins the binding arbitrarily — in the same import, one
pair bound the note's copy and the other bound the orphan. Both are parented to
the journal row, so both show in the app's Unplanned list and neither shows up
over MCP. The failure is silent and permanent: the calendar block you drag out
of the orphan is not the task your checkbox completes.

**And `createTask(parent = <journal row>)` does not insert a block.** The
relationship is one-way — a todo block mints a task, a parented task does not
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

## The agent path, and what it costs

Quick-create splits by how the input was submitted, not by guessing at it:

| Key | Behaviour |
| --- | --- |
| `Enter` | sigil templates, deterministic, no model. `x …` → task to Inbox; `> …` → task scheduled today; anything else → paragraph appended to today's journal |
| `Ctrl-P` | headless Claude proposes a plan; you approve it |

Sigils are a decision; cue-detection heuristics are a guess. Enter must never
cost a model call or a wait.

The Ctrl-P path runs `claude -p` against the user's existing auth (no API key to
manage), and the shape that matters is **the model gets no tools at all**. The
helper pre-fetches a bounded context and the model returns a plan as JSON, which
the helper validates against the real tool schemas and executes. Measured on one
representative input:

| Config | Wall | Turns | Cost | Cache read |
| --- | --- | --- | --- | --- |
| Fable 5.1 + 52 MCP tools | 39s | 11 | $0.71 | 187k |
| Haiku 4.5 + 52 MCP tools | 37s | 6 | $0.056 | 113k |
| **Haiku, no tools, pre-fetched context** | **13.7s** | **1** | **$0.019** | **17.8k** |

Latency is the harness, not inference — Haiku with tools was no faster than
Fable. The win comes from deleting the tool round trips: the helper does in
~80ms of local HTTP what the agent spent six model turns on. The remaining ~13s
is `claude -p` startup and is the floor short of dropping to the SDK, which
would cost the no-API-key property.

Two things the no-tools model got wrong, both handled by the validator rather
than by prompting harder: it flattened `time` into top-level
`start_time`/`end_time` (no tool schema to copy), and it fenced its JSON in
` ```json `. Model output is another untrusted string arriving at the shell.

Giving it the read tools did buy real judgement — it disambiguated one of four
contacts sharing a first name by reading which project it was about. Haiku without tools did not.
If that is wanted back, the helper should do the contact search and put the
candidates in the context, not hand over the tool.

## House rules

Inherited from `riclib.capacities`, which earned them under marketplace review.

- **The QML never talks to the API.** Token, bounding, caching and queueing live
  in `bin/`. A credential does not belong in the shell's process state.
- **Nothing polls.** A user action refreshes. The one live thing is the
  countdown ring, and it is local arithmetic on a single fetched timestamp — the
  data costs one call, the ticking costs nothing.
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

## Gotchas that cost an afternoon elsewhere

Carried over from `riclib.capacities` — all of these apply here too.

- **A bar widget component is cached by URL.** Editing `BarSlot.qml` in place
  does nothing, not even with `omarchy-shell shell rescanPlugins`. Only
  `omarchy restart shell` picks it up. Overlays *do* hot-reload, which makes
  this easy to misdiagnose.
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
