//! Everything the widget needs, in one call.
//!
//! The overlay should not be making four requests and reconciling them itself,
//! and it certainly should not be deciding what is safe to render. Both jobs
//! belong here: this is the boundary, so this is where things are bounded.
//!
//! Two of the shapes are not obvious and are worth knowing before changing
//! anything:
//!
//!   * **Today is a union.** A task anchored only by a parent on the journal
//!     row — every checkbox typed into the daily note — is invisible to
//!     `listTodaysTasks`, while the app shows it in Today. So the list is the
//!     note's todo blocks *plus* the tasks scheduled for today, deduplicated.
//!
//!   * **A task blocked on the calendar is an event, and only an event.** It
//!     leaves `listTodaysTasks` entirely and comes back through
//!     `listEventsForDateRange` with nothing in the list entry to say it is
//!     not a meeting; `getEvent` carries `allocationOfTask`. So the day is
//!     drawn as an *agenda* — meetings and blocks together, in time order,
//!     each with its kind — and a task that is in the agenda is not also in
//!     the list. The countdown counts to the next agenda item of either
//!     kind: a block is time you committed, and it deserves the same ring.
//!
//!   * **An event description is hostile.** A Teams invite is thousands of
//!     characters of dial-in numbers, legal text and angle-bracketed links. The
//!     one useful thing in it is the join URL, so that is extracted here
//!     against an allowlist of hosts and the description itself never leaves.

use crate::journal::{self, Journal};
use crate::mcp::{self, Client};
use serde_json::{json, Value};
use time::{Date, OffsetDateTime};

/// Bounds, applied on the way out. A stale cache written by an older build is
/// then bounded on read too, which clamping at each sink would not give us.
const MAX_EVENTS: usize = 20;
const MAX_TASKS: usize = 40;
const MAX_TITLE: usize = 160;
const MAX_URL: usize = 400;

/// Hosts whose links we are willing to hand to `xdg-open`. An event's
/// description is written by whoever sent the invite, so the question is not
/// "is this a URL" but "is this a URL we chose to trust".
const MEETING_HOSTS: &[(&str, &str)] = &[
    ("teams.microsoft.com", "teams"),
    ("teams.live.com", "teams"),
    ("zoom.us", "zoom"),
    ("meet.google.com", "meet"),
    ("webex.com", "webex"),
    ("whereby.com", "whereby"),
    ("meet.jit.si", "jitsi"),
];

fn clamp(s: &str, n: usize) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= n {
        return collapsed;
    }
    collapsed.chars().take(n).collect()
}

/// The first URL in `text` whose host is one we trust, with the platform it
/// belongs to. Angle brackets matter: invites wrap links as `<https://…>`, so a
/// scanner that only stops at whitespace swallows the closing bracket.
fn join_link(text: &str) -> Option<(String, &'static str)> {
    let mut rest = text;
    while let Some(at) = rest.find("https://") {
        let tail = &rest[at..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '>' | '<' | '"' | ')' | '|'))
            .unwrap_or(tail.len());
        let url = &tail[..end];
        let host = url
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        for (needle, platform) in MEETING_HOSTS {
            // Match the host, not the URL: a link to evil.com/teams.microsoft.com
            // is not a Teams meeting.
            if host == *needle || host.ends_with(&format!(".{needle}")) {
                if url.len() <= MAX_URL {
                    return Some((url.to_owned(), platform));
                }
            }
        }
        rest = &tail[end.max(1)..];
    }
    None
}

/// What kind of meeting this is, from the cheap signal first. `location` is a
/// short server-chosen string ("Microsoft Teams Meeting"); the description is
/// only consulted for the link.
fn platform(location: &str, from_link: Option<&'static str>) -> Option<&'static str> {
    let l = location.to_ascii_lowercase();
    for (_, name) in MEETING_HOSTS {
        if l.contains(name) {
            return Some(name);
        }
    }
    if l.contains("google meet") {
        return Some("meet");
    }
    from_link
}

fn events_for(client: &Client, day: Date) -> mcp::Result<Vec<Value>> {
    let d = day.to_string();
    let got = client.call(
        "personal_events_listEventsForDateRange",
        json!({ "start_date": d, "end_date": d, "filter": "all", "limit": 50 }),
    )?;
    let mut items = got.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    // Unsorted from the API, always. Anything calling one of these "next" has
    // to sort first or it lies.
    items.sort_by_key(|e| {
        e.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("").to_owned()
    });
    items.truncate(MAX_EVENTS);
    Ok(items)
}

/// One agenda item, reduced to what a widget may render. `full` is the
/// `getEvent` detail: its description is read for a join link and dropped,
/// and `allocationOfTask` is what makes it a block rather than a meeting.
/// `completed` is the block's task state, looked up by the caller.
fn reduce_event(raw: &Value, full: &Value, completed: Option<bool>) -> Value {
    let id = raw.get("id").and_then(Value::as_str).unwrap_or("");
    let start = raw.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("");
    let end = raw.pointer("/time/end_time").and_then(Value::as_str).unwrap_or("");
    let task = full.get("allocationOfTask").and_then(Value::as_str).unwrap_or("");

    let location = full.get("location").and_then(Value::as_str).unwrap_or("");
    let description = full.get("description").and_then(Value::as_str).unwrap_or("");
    let link = join_link(description).or_else(|| join_link(location));
    let (join, from_link) = match &link {
        Some((u, p)) => (Some(u.clone()), Some(*p)),
        None => (None, None),
    };

    let mut item = json!({
        "id": id,
        "kind": if task.is_empty() { "meeting" } else { "block" },
        "title": clamp(raw.get("title").and_then(Value::as_str).unwrap_or(""), MAX_TITLE),
        "start": start,
        "end": end,
        "at": start.get(11..16).unwrap_or(""),
        "length": minutes_between(start, end).unwrap_or(0).max(0),
        "platform": platform(location, from_link),
        "join": join,
    });
    if !task.is_empty() {
        item["task"] = json!(task);
        item["done"] = json!(completed.unwrap_or(false));
    }
    item
}

/// Minutes between two same-day `…Thh:mm:ss` stamps, by the clock face.
fn minutes_between(now: &str, then: &str) -> Option<i64> {
    let mins = |s: &str| -> Option<i64> {
        Some(s.get(11..13)?.parse::<i64>().ok()? * 60 + s.get(14..16)?.parse::<i64>().ok()?)
    };
    Some(mins(then)? - mins(now)?)
}

pub fn build(client: &Client, jrn: &Journal) -> Result<(String, Value), String> {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let day = now.date();
    let stamp = now
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second]"
        ))
        .unwrap_or_default();

    // --- The agenda: every timed thing on the day, meetings and blocks alike.
    // --- Each wants its detail: the join link for a meeting, the task for a
    // --- block. Twenty local reads at a millisecond each.
    let raw_events = events_for(client, day).map_err(|e| e.to_string())?;
    let mut agenda: Vec<Value> = Vec::new();
    let mut blocked: Vec<String> = Vec::new();
    for raw in &raw_events {
        let id = raw.get("id").and_then(Value::as_str).unwrap_or("");
        let full = client
            .call("personal_events_getEvent", json!({ "event": id }))
            .unwrap_or(Value::Null);
        let task = full.get("allocationOfTask").and_then(Value::as_str).unwrap_or("");
        let completed = if task.is_empty() {
            None
        } else {
            blocked.push(task.to_owned());
            client
                .call("tasks_getTask", json!({ "task": task }))
                .ok()
                .and_then(|t| t.get("completed").and_then(Value::as_bool))
        };
        agenda.push(reduce_event(raw, &full, completed));
    }

    let next = agenda
        .iter()
        .find(|e| e["start"].as_str().unwrap_or("") > stamp.as_str())
        .map(|e| {
            let mut e = e.clone();
            let mins = minutes_between(&stamp, e["start"].as_str().unwrap_or("")).unwrap_or(0);
            e["minutes"] = json!(mins);
            e
        });

    // --- Today's tasks: the note's checkboxes, then anything scheduled that
    // --- is not already among them — and nothing that is already a block,
    // --- which is drawn in the agenda instead.
    let (row, _) = jrn.row_for(client, day).map_err(|e| e.to_string())?;
    let doc = mcp::unwrap(
        &client.call("tables_getObject", json!({ "object": row })).map_err(|e| e.to_string())?,
    );
    let mut tasks: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    if let Some(blocks) = doc.pointer("/data/Notes/blocks").and_then(Value::as_array) {
        for b in blocks.iter().filter(|b| b["type"] == "todo") {
            let id = b.get("task").and_then(Value::as_str).unwrap_or("").to_owned();
            if !id.is_empty() {
                seen.push(id.clone());
            }
            if blocked.contains(&id) {
                continue;
            }
            tasks.push(json!({
                "id": id,
                "title": clamp(b.get("content").and_then(Value::as_str).unwrap_or(""), MAX_TITLE),
                "done": b.get("checked").and_then(Value::as_bool).unwrap_or(false),
                "source": "journal",
            }));
        }
    }
    let scheduled = client
        .call("tasks_listTodaysTasks", json!({ "workspace": jrn.workspace, "limit": 50 }))
        .map_err(|e| e.to_string())?;
    for t in scheduled.pointer("/todo/items").and_then(Value::as_array).unwrap_or(&vec![]) {
        let id = t.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        if seen.contains(&id) || blocked.contains(&id) {
            continue;
        }
        tasks.push(json!({
            "id": id,
            "title": clamp(t.get("title").and_then(Value::as_str).unwrap_or(""), MAX_TITLE),
            "done": false,
            "source": "scheduled",
        }));
    }
    tasks.truncate(MAX_TASKS);
    // Open counts blocks too: a block is a task with a time, not a meeting.
    let open = tasks.iter().filter(|t| t["done"] == false).count()
        + agenda.iter().filter(|e| e["kind"] == "block" && e["done"] == false).count();

    let payload = json!({
        "date": day.to_string(),
        "title": journal::title_for(day),
        "now": stamp,
        "next": next,
        "agenda": agenda,
        "tasks": tasks,
        "open": open,
    });

    // The human rendering is a courtesy; --json is what this command is for.
    let mut out = format!("# {}  —  {open} open\n\n", journal::title_for(day));
    match &payload["next"] {
        Value::Null => out += "*nothing else today*\n\n",
        n => {
            out += &format!(
                "**{}** {}{}\n\n",
                n["at"].as_str().unwrap_or(""),
                n["title"].as_str().unwrap_or(""),
                n["join"].as_str().map(|_| "  *(joinable)*").unwrap_or(""),
            );
        }
    }
    if !agenda.is_empty() {
        out += "## Scheduled\n\n";
        for e in payload["agenda"].as_array().unwrap_or(&vec![]) {
            let mark = match (e["kind"].as_str(), e["done"].as_bool()) {
                (Some("block"), Some(true)) => "[x] ",
                (Some("block"), _) => "[ ] ",
                _ => "",
            };
            out += &format!(
                "- **{}** {mark}{}  *({}m)*\n",
                e["at"].as_str().unwrap_or(""),
                e["title"].as_str().unwrap_or(""),
                e["length"].as_i64().unwrap_or(0),
            );
        }
        out += "\n";
    }
    out += "## Anytime\n\n";
    for t in payload["tasks"].as_array().unwrap_or(&vec![]) {
        out += &format!(
            "- [{}] {}\n",
            if t["done"] == true { "x" } else { " " },
            t["title"].as_str().unwrap_or("")
        );
    }
    Ok((out, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shaped like a real Teams invite — the join link buried among dial-in
    // numbers, an untrusted aka.ms link first, and the useful one wrapped in
    // angle brackets. Every identifier here is invented.
    const TEAMS: &str = "________________\nMicrosoft Teams meeting\nJoin: \
        https://teams.microsoft.com/meet/000000000000000?p=EXAMPLEEXAMPLE\n\
        Meeting ID: 000 000 000 000\nPasscode: xxxxxxxx\n\
        Need help?<https://aka.ms/JoinTeamsMeeting?omkt=en-US> | System reference\
        <https://teams.microsoft.com/l/meetup-join/19%3ameeting_EXAMPLE@thread.v2/0>\n\
        Dial in by phone +1 555-000-0000,,000000000#";

    #[test]
    fn the_join_link_comes_out_of_the_blob() {
        let (url, platform) = join_link(TEAMS).unwrap();
        assert_eq!(platform, "teams");
        assert_eq!(url, "https://teams.microsoft.com/meet/000000000000000?p=EXAMPLEEXAMPLE");
    }

    #[test]
    fn an_angle_bracket_ends_a_url() {
        // aka.ms is not on the allowlist, so the next candidate is the
        // bracketed teams link -- and it must not swallow the closing '>'.
        let wrapped = "ref <https://teams.microsoft.com/l/meetup-join/19> and more";
        let (url, _) = join_link(wrapped).unwrap();
        assert_eq!(url, "https://teams.microsoft.com/l/meetup-join/19");
    }

    #[test]
    fn an_untrusted_host_is_not_a_meeting() {
        assert!(join_link("https://example.com/zoom.us/j/123").is_none());
        assert!(join_link("come to https://aka.ms/JoinTeamsMeeting").is_none());
    }

    #[test]
    fn a_lookalike_host_does_not_pass() {
        // The host must be the trusted one or a subdomain of it, not a string
        // that merely contains it.
        assert!(join_link("https://teams.microsoft.com.evil.test/x").is_none());
        assert!(join_link("https://eu.zoom.us/j/999").is_some());
    }

    #[test]
    fn the_platform_comes_from_location_when_there_is_one() {
        assert_eq!(platform("Microsoft Teams Meeting", None), Some("teams"));
        assert_eq!(platform("Google Meet", None), Some("meet"));
        assert_eq!(platform("Room 3", Some("zoom")), Some("zoom"));
        assert_eq!(platform("Room 3", None), None);
    }

    #[test]
    fn a_block_is_told_from_a_meeting_by_its_task() {
        let raw = json!({"id": "event:1", "title": "Pick up car\n", "time": {
            "start_time": "2026-09-03T14:00:00+03:00", "end_time": "2026-09-03T15:00:00+03:00"}});
        let block = reduce_event(&raw, &json!({"allocationOfTask": "task:9"}), Some(false));
        assert_eq!(block["kind"], "block");
        assert_eq!(block["task"], "task:9");
        assert_eq!(block["done"], false);
        assert_eq!(block["length"], 60);
        assert_eq!(block["at"], "14:00");
        assert_eq!(block["title"], "Pick up car");

        let meeting = reduce_event(&raw, &json!({"location": "Microsoft Teams Meeting", "description": TEAMS}), None);
        assert_eq!(meeting["kind"], "meeting");
        assert!(meeting.get("task").is_none(), "a meeting has no task and no done");
        assert!(meeting.get("done").is_none());
        assert_eq!(meeting["platform"], "teams");
        assert!(meeting["join"].as_str().unwrap().starts_with("https://teams.microsoft.com/meet/"));

        // No detail at all — getEvent failed — is still a meeting, drawn plainly.
        let plain = reduce_event(&raw, &Value::Null, None);
        assert_eq!(plain["kind"], "meeting");
        assert_eq!(plain["join"], Value::Null);
    }

    #[test]
    fn titles_are_collapsed_and_clamped() {
        assert_eq!(clamp("\nAutomate the\n  thing\n", 160), "Automate the thing");
        assert_eq!(clamp(&"x".repeat(400), 160).chars().count(), 160);
    }
}
