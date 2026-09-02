//! rtn — a command line for Routine, over its local MCP server.
//!
//! Two audiences, one surface: a person typing `rtn log "shipped the ring"`,
//! and a program — an agent, or the Omarchy shell widget — asking for `--json`.
//! Nothing else in this project talks to MCP; if a credential, a bound or a
//! retry is involved, it belongs here rather than in QML or a prompt.

mod journal;
mod mcp;
mod md;

use clap::{Args, Parser, Subcommand};
use md::Checkbox;
use serde_json::{json, Value};
use std::io::{IsTerminal, Read};
use time::{Date, OffsetDateTime};

#[derive(Parser)]
#[command(name = "rtn", version, about = "A command line for Routine", max_term_width = 84)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Append to a daily log entry
    ///
    /// Text may be an argument or arrive on stdin, and is read as markdown:
    /// headings, bullets, quotes and checkboxes become the blocks they look
    /// like, and a fenced code block stays one block however long it is.
    ///
    ///   rtn log "just had a ball"
    ///   rtn log --task pick up the milk
    ///   cat notes.md | rtn log
    Log(Log),
    /// What today holds: the next event, and the tasks in play
    Today {
        #[arg(long)]
        json: bool,
    },
    /// The next event, and how long there is before it
    Next {
        #[arg(long)]
        json: bool,
    },
    /// Check the connection and say what is wrong when there is something wrong
    Doctor,
}

#[derive(Args)]
struct Log {
    /// The text. Omit it to read stdin.
    text: Vec<String>,
    /// Log it as a task rather than a note
    #[arg(long, short = 't')]
    task: bool,
    /// The day to write to, as YYYY-MM-DD. Defaults to today.
    #[arg(long, value_name = "YYYY-MM-DD")]
    date: Option<String>,
    /// Checkboxes become inert boxes rather than live tasks. For backfilling
    /// old days, whose checkboxes are mostly long since settled.
    #[arg(long)]
    historical: bool,
    /// Show what would be written, and write nothing
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

fn main() {
    if let Err(message) = run() {
        eprintln!("rtn: {message}");
        std::process::exit(1);
    }
}

fn today() -> Date {
    OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
        .date()
}

fn parse_date(s: &str) -> Result<Date, String> {
    let f = time::macros::format_description!("[year]-[month]-[day]");
    Date::parse(s, &f).map_err(|_| format!("{s} is not a date; use YYYY-MM-DD"))
}

fn run() -> Result<(), String> {
    match Cli::parse().command {
        Command::Log(args) => log(args),
        Command::Today { json } => today_cmd(json),
        Command::Next { json } => next_cmd(json),
        Command::Doctor => doctor(),
    }
}

fn log(args: Log) -> Result<(), String> {
    let mut text = args.text.join(" ");
    if text.is_empty() {
        if std::io::stdin().is_terminal() {
            return Err("nothing to log: give it text, or pipe some in".into());
        }
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| format!("could not read stdin: {e}"))?;
    }
    if text.trim().is_empty() {
        return Err("nothing to log: the input was empty".into());
    }

    let day = match &args.date {
        Some(d) => parse_date(d)?,
        None => today(),
    };

    // --task means the whole input is one task, whatever it looks like. Only a
    // checkbox typed as markdown goes through the mode below.
    let blocks = if args.task {
        vec![md::Block::Todo { content: text.trim().to_owned(), checked: false }]
    } else {
        md::parse(&text, if args.historical { Checkbox::Inert } else { Checkbox::Task })
    };

    if args.dry_run {
        if args.json {
            let doc: Vec<Value> = blocks.iter().map(|b| b.to_json(None)).collect();
            println!("{}", serde_json::to_string_pretty(&doc).unwrap());
        } else {
            println!("{} — would write {} block(s):", journal::title_for(day), blocks.len());
            for b in &blocks {
                let j = b.to_json(None);
                let kind = j["type"].as_str().unwrap_or("?");
                let content = j["content"].as_str().unwrap_or("");
                let head = content.lines().next().unwrap_or("");
                println!("  {kind:<10} {}", truncate(head, 66));
            }
        }
        return Ok(());
    }

    let client = mcp::Client::connect().map_err(|e| e.to_string())?;
    let jrn = journal::Journal::discover(&client).map_err(|e| e.to_string())?;
    let done = journal::append(&client, &jrn, day, &blocks).map_err(|e| e.to_string())?;

    if args.json {
        println!(
            "{}",
            json!({
                "date": day.to_string(),
                "blocks": done.written,
                "tasks": done.tasks,
                "created_day": done.created_day,
            })
        );
    } else {
        let mut line = format!("{} — {} block(s)", journal::title_for(day), done.written);
        if !done.tasks.is_empty() {
            line += &format!(", {} task(s)", done.tasks.len());
        }
        if done.created_day {
            line += " (day created)";
        }
        println!("{line}");
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_owned();
    }
    s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
}

fn today_cmd(as_json: bool) -> Result<(), String> {
    let client = mcp::Client::connect().map_err(|e| e.to_string())?;
    let jrn = journal::Journal::discover(&client).map_err(|e| e.to_string())?;
    let day = today();

    let events = sorted_events(&client, day)?;
    let tasks = client
        .call("tasks_listTodaysTasks", json!({ "workspace": jrn.workspace, "limit": 50 }))
        .map_err(|e| e.to_string())?;

    if as_json {
        println!("{}", json!({ "date": day.to_string(), "events": events, "tasks": tasks }));
        return Ok(());
    }

    println!("{}", journal::title_for(day));
    if events.is_empty() {
        println!("  no events");
    }
    for e in &events {
        let start = e.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("");
        let title = e.get("title").and_then(Value::as_str).unwrap_or("");
        println!("  {}  {}", &start.get(11..16).unwrap_or("     "), truncate(collapse(title).trim(), 60));
    }
    let open = tasks.pointer("/todo/items").and_then(Value::as_array).cloned().unwrap_or_default();
    println!("\n  {} task(s) scheduled today", open.len());
    for t in &open {
        println!("  [ ] {}", truncate(collapse(t["title"].as_str().unwrap_or("")).trim(), 66));
    }
    Ok(())
}

/// Titles arrive with newlines in them, straight from the API.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sorted_events(client: &mcp::Client, day: Date) -> Result<Vec<Value>, String> {
    let d = day.to_string();
    let got = client
        .call(
            "personal_events_listEventsForDateRange",
            json!({ "start_date": d, "end_date": d, "filter": "all", "limit": 50 }),
        )
        .map_err(|e| e.to_string())?;
    let mut items = got.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    // The API returns them in no order at all — Sep 2, Sep 5, Sep 4, Sep 2 for
    // one range. Anything calling this "next" has to sort first.
    items.sort_by_key(|e| {
        e.pointer("/time/start_time")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    });
    Ok(items)
}

fn next_cmd(as_json: bool) -> Result<(), String> {
    let client = mcp::Client::connect().map_err(|e| e.to_string())?;
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let events = sorted_events(&client, now.date())?;

    let stamp = now
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second]"
        ))
        .unwrap_or_default();
    let next = events.iter().find(|e| {
        e.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("") > stamp.as_str()
    });

    match next {
        None => {
            if as_json {
                println!("{}", json!({ "next": Value::Null }));
            } else {
                println!("nothing else today");
            }
        }
        Some(e) => {
            let start = e.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("");
            let title = collapse(e.get("title").and_then(Value::as_str).unwrap_or(""));
            if as_json {
                println!("{}", json!({ "next": { "title": title, "start": start } }));
            } else {
                println!("{}  {}", start.get(11..16).unwrap_or(""), title.trim());
            }
        }
    }
    Ok(())
}

fn doctor() -> Result<(), String> {
    let path = mcp::token_path();
    println!("token   {}", path.display());
    match mcp::token() {
        Ok(t) => println!("        found, {} chars", t.len()),
        Err(e) => return Err(e.to_string()),
    }
    let client = mcp::Client::connect().map_err(|e| e.to_string())?;
    let jrn = journal::Journal::discover(&client).map_err(|e| e.to_string())?;
    println!("server  answering on 127.0.0.1:8765");
    println!("space   {}\njournal {}", jrn.workspace, jrn.table);
    Ok(())
}
