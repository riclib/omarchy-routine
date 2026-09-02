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

/// One event, reduced to what a widget may render. The description is read for
/// a join link and then dropped.
fn reduce_event(client: &Client, raw: &Value, detail: bool) -> Value {
    let id = raw.get("id").and_then(Value::as_str).unwrap_or("");
    let start = raw.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("");
    let end = raw.pointer("/time/end_time").and_then(Value::as_str).unwrap_or("");

    // Only the next event is worth a second call; the rest of the day does not
    // need a join button.
    let full = if detail {
        client.call("personal_events_getEvent", json!({ "event": id })).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let location = full.get("location").and_then(Value::as_str).unwrap_or("");
    let description = full.get("description").and_then(Value::as_str).unwrap_or("");
    let link = join_link(description).or_else(|| join_link(location));
    let (join, from_link) = match &link {
        Some((u, p)) => (Some(u.clone()), Some(*p)),
        None => (None, None),
    };

    json!({
        "id": id,
        "title": clamp(raw.get("title").and_then(Value::as_str).unwrap_or(""), MAX_TITLE),
        "start": start,
        "end": end,
        "at": start.get(11..16).unwrap_or(""),
        "platform": platform(location, from_link),
        "join": join,
    })
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

    let raw_events = events_for(client, day).map_err(|e| e.to_string())?;
    let next_index = raw_events.iter().position(|e| {
        e.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("") > stamp.as_str()
    });
    let events: Vec<Value> = raw_events
        .iter()
        .enumerate()
        .map(|(i, e)| reduce_event(client, e, Some(i) == next_index))
        .collect();

    let next = next_index.map(|i| {
        let mut e = events[i].clone();
        let mins = minutes_between(&stamp, e["start"].as_str().unwrap_or("")).unwrap_or(0);
        e["minutes"] = json!(mins);
        e
    });

    // --- Today's tasks: the note's checkboxes, then anything scheduled that
    // --- is not already among them.
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
        if seen.contains(&id) {
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
    let open = tasks.iter().filter(|t| t["done"] == false).count();

    let payload = json!({
        "date": day.to_string(),
        "title": journal::title_for(day),
        "now": stamp,
        "next": next,
        "events": events,
        "tasks": tasks,
        "open": open,
    });

    // The human rendering is a courtesy; --json is what this command is for.
    let mut out = format!("# {}\n\n", journal::title_for(day));
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
    out += &format!("## Tasks — {open} open\n\n");
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

    const TEAMS: &str = "________________\nMicrosoft Teams meeting\nJoin: \
        https://teams.microsoft.com/meet/000000000000000?p=EXAMPLEEXAMPLE\n\
        Meeting ID: 000 000 000 000\nPasscode: xxxxxxxx\n\
        Need help?<https://aka.ms/JoinTeamsMeeting?omkt=en-US> | System reference\
        <https://teams.microsoft.com/l/meetup-join/19%3ameeting_ZDc5@thread.v2/0>\n\
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
    fn titles_are_collapsed_and_clamped() {
        assert_eq!(clamp("\nAutomate the\n  thing\n", 160), "Automate the thing");
        assert_eq!(clamp(&"x".repeat(400), 160).chars().count(), 160);
    }
}
