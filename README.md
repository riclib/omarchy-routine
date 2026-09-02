# omarchy-routine

An Omarchy shell plugin for [Routine](https://routine.co) — the dashboard as a
Quickshell overlay, a countdown to the next meeting in the bar, and capture
straight into today's journal.

**Status: nothing built yet.** The data layer underneath is mapped and measured;
see `CLAUDE.md` for how Routine's MCP server actually behaves, including several
findings that are not in its documentation.

## Why

Routine's own dashboard has the right shape — a ring counting down to the next
event, a NEXT card, today's tasks, a quick-create box — and takes anywhere
between instant and a minute to load. Sometimes it never does. Everything it
renders is reachable over Routine's local MCP server, so the widget can be an
overlay that is simply always there.

## Shape

- `overlay` — the dashboard: countdown ring, next event, today's tasks, capture
- `bar-widget` — the always-on version of the ring
- `bin/` — the helper: holds the token, bounds every response, owns the queue

Capture splits on the key you press. `Enter` is deterministic and instant:
`x …` files a task to the Inbox, `> …` schedules one for today, anything else
appends a paragraph to today's journal. `Ctrl-P` asks a headless Claude for a
plan and shows it to you before anything is written.

## Requires

Routine, with its MCP server switched on in Settings → MCP (it is off by
default). The `Ctrl-P` path additionally wants `claude` on `PATH`; everything
else works without it.
