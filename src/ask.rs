//! `rtn ask` — a question about your Routine, answered by a headless agent.
//!
//! Runs the `claude` CLI already on the machine, so it uses whatever auth the
//! user has and there is no API key to manage. The Routine MCP config is
//! generated from the token file at call time and handed over with
//! `--strict-mcp-config`, so the agent sees Routine's tools and nothing else —
//! not the user's other MCP servers, and none of Claude Code's own file or
//! shell tools.
//!
//! **The tool list is an allowlist, and it is the security boundary.** This is
//! an agent reachable from a bar overlay, acting on real data, so it can read
//! anything and create or amend a task — and it cannot delete, cannot alter a
//! table's shape, cannot touch other workspaces, and cannot send a notice to
//! another person.

use crate::mcp;
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

/// Omarchy keeps the user's chosen coding agent in a file, and `omarchy default
/// agent` reads it. That convention says *which* agent; it does not say how to
/// drive one headlessly, and there is no shared contract for that — each CLI
/// differs, and so does how MCP servers reach it.
///
/// So this honours the choice for picking a driver, and is explicit when the
/// chosen agent has none yet rather than silently running something else.
fn omarchy_default_agent() -> Option<String> {
    let path = std::path::PathBuf::from(std::env::var("HOME").ok()?)
        .join(".config/omarchy/defaults/agent");
    let name = std::fs::read_to_string(path).ok()?.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// Agents `rtn ask` knows how to run. Claude is here because it takes its MCP
/// config per invocation (`--mcp-config` with `--strict-mcp-config`), which is
/// what lets the agent be handed Routine and nothing else — that scoping is the
/// security boundary, not a convenience.
///
/// Others are drivable in principle — `grok agent`, `codex exec`,
/// `opencode run` are all headless — but they take MCP servers from persistent
/// config rather than per call, so pointing one at Routine means writing into
/// the user's own agent config and inheriting whatever else is in it. That is a
/// different bargain and wants deciding rather than assuming.
const DRIVERS: &[&str] = &["claude"];

/// Everything the agent may call. Read tools are broad; writes are the two a
/// person would ask for out loud, and nothing that destroys or notifies.
const ALLOWED: &[&str] = &[
    // read
    "mcp__routine__search_search",
    "mcp__routine__tasks_listTasks",
    "mcp__routine__tasks_listTodaysTasks",
    "mcp__routine__tasks_listUnplannedTasks",
    "mcp__routine__tasks_getTask",
    "mcp__routine__personal_events_listEventsForDateRange",
    "mcp__routine__personal_events_getEvent",
    "mcp__routine__personal_events_findAvailableTimeSlots",
    "mcp__routine__personal_contacts_searchContacts",
    "mcp__routine__personal_contacts_listContacts",
    "mcp__routine__personal_pages_listPages",
    "mcp__routine__personal_pages_getPage",
    "mcp__routine__personal_calendars_listCalendars",
    "mcp__routine__tables_listTables",
    "mcp__routine__tables_getTableSchema",
    "mcp__routine__tables_searchTableRows",
    "mcp__routine__tables_getObject",
    "mcp__routine__buildLink",
    // write — deliberately only these two
    "mcp__routine__tasks_createTask",
    "mcp__routine__tasks_updateTask",
];

fn system_prompt(now: &str, workspace: &str, context: &str) -> String {
    // The field notes, as a briefing. Everything here cost time to find, and an
    // agent that does not know it makes exactly the mistakes we already made.
    format!(
        "You are an assistant for the user's Routine (routine.co) workspace, reached \
from a desktop bar. It is now {now}. The personal workspace is {workspace}.\n\
\n\
TODAY, already fetched for you — answer from this when it is enough, and only \
reach for a tool when the question needs something it does not contain:\n\
{context}\n\
\n\
Answer in at most a short paragraph unless asked for more. Prefer doing over \
explaining. You are on a small overlay, so be brief and concrete: name times, \
titles and counts rather than describing them.\n\
\n\
Things about this API that are not obvious, and that you should not rediscover:\n\
- Events come back UNSORTED from listEventsForDateRange. Sort before saying \
'next' or 'first'.\n\
- Today's tasks are a UNION: listTodaysTasks misses any task whose only anchor \
is a parent on today's journal row, which is every checkbox typed into the \
daily note. If asked what is on today, check both.\n\
- listTasks is id and title only, truncated, unordered. It is an index, not a \
payload; use getTask for detail.\n\
- 'scheduled' is a bare string: a date for a day, YYYY-WW for a week batch.\n\
- Scheduling is one-way here: you can set a schedule and cannot remove one. Do \
not promise to unplan something.\n\
- Create tasks UNPLANNED unless the user gave a day. The parent already records \
when it was captured.\n\
\n\
You may read freely, and you may create or amend a task. You cannot delete \
anything, change a table's shape, or message another person — if asked, say so \
plainly rather than trying. Confirm what you changed in one line."
    )
}

pub fn run(question: &str, model: Option<&str>, agent: Option<&str>) -> Result<(String, Value), String> {
    if question.trim().is_empty() {
        return Err("ask what?".into());
    }

    // Explicit flag, else Omarchy's default, else claude.
    let chosen = agent
        .map(str::to_owned)
        .or_else(omarchy_default_agent)
        .unwrap_or_else(|| "claude".into());
    if !DRIVERS.contains(&chosen.as_str()) {
        return Err(format!(
            "your Omarchy default agent is `{chosen}`, which rtn cannot drive headlessly yet \
             (it knows: {}).\n\
             Either point this at one it knows — `rtn ask --agent claude`, or the plugin's \
             `askAgent` setting — or change the default with `omarchy default agent claude`.\n\
             Driving `{chosen}` needs Routine registered in its own MCP config rather than \
             passed per call, which is a different trade and not yet made.",
            DRIVERS.join(", ")
        ));
    }
    // The agent gets Routine and nothing else, so the config is built here
    // rather than borrowed from whatever the user has registered.
    let token = mcp::token().map_err(|e| e.to_string())?;
    let config = json!({
        "mcpServers": { "routine": {
            "type": "http",
            "url": "http://127.0.0.1:8765/mcp",
            "headers": { "Authorization": format!("Bearer {token}") },
        }}
    });

    let dir = std::env::temp_dir().join(format!("rtn-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make a temp dir: {e}"))?;
    let path = dir.join("routine.json");
    {
        let mut file = std::fs::File::create(&path)
            .map_err(|e| format!("could not write the MCP config: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        file.write_all(serde_json::to_string(&config).unwrap().as_bytes())
            .map_err(|e| format!("could not write the MCP config: {e}"))?;
    }

    let jrn_workspace = crate::mcp::Client::connect()
        .and_then(|c| crate::journal::Journal::discover(&c).map(|j| j.workspace))
        .unwrap_or_else(|_| "the personal workspace".into());
    // Priming the prompt with the day costs 10ms and saves the agent several
    // model round trips, which are the whole latency. Most questions asked of a
    // bar overlay are about today and need no tool call at all.
    let (now, context) = crate::ask_context();

    let mut command = Command::new("claude");
    command
        .arg("-p")
        .arg(question)
        .arg("--mcp-config")
        .arg(&path)
        .arg("--strict-mcp-config")
        .arg("--append-system-prompt")
        .arg(system_prompt(&now, &jrn_workspace, &context))
        .arg("--output-format")
        .arg("json")
        .arg("--allowedTools");
    for tool in ALLOWED {
        command.arg(tool);
    }
    if let Some(m) = model {
        command.arg("--model").arg(m);
    }

    let out = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run `claude`: {e}\nIs it installed and on PATH?"))?;

    // The config carried a bearer token; it does not outlive the call.
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "claude exited {}: {}",
            out.status.code().unwrap_or(-1),
            err.lines().next().unwrap_or("no detail")
        ));
    }

    let doc: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("could not read claude's reply: {e}"))?;
    let answer = doc
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("no answer")
        .trim()
        .to_owned();

    let payload = json!({
        "question": question,
        "answer": answer,
        "cost_usd": doc.get("total_cost_usd"),
        "duration_ms": doc.get("duration_ms"),
        "turns": doc.get("num_turns"),
    });
    Ok((answer, payload))
}
