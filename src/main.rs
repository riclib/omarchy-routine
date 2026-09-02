//! rtn — a command line for Routine, over its local MCP server.
//!
//! Two audiences, one surface: a person typing `rtn log "shipped the ring"`,
//! and a program — an agent, or the Omarchy shell widget — asking for `--json`.
//! Nothing else in this project talks to MCP; if a credential, a bound or a
//! retry is involved, it belongs here rather than in QML or a prompt.

mod auth;
mod journal;
mod mcp;
mod md;
mod render;

use clap::{Args, Parser, Subcommand};
use md::Checkbox;
use render::Format;
use serde_json::{json, Value};
use std::io::{IsTerminal, Read};
use time::{Date, OffsetDateTime};

#[derive(Parser)]
#[command(name = "rtn", version, about = "A command line for Routine", max_term_width = 84)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    #[command(flatten)]
    format: FormatArgs,
}

/// How to say it. Pretty when a person is watching, plain when piped, and
/// --json when the reader is a program. `--md` exists so output composes:
/// `rtn today --md` is valid input to `rtn log`.
#[derive(Args, Default)]
#[group(multiple = false)]
struct FormatArgs {
    /// Rendered for a terminal (the default when stdout is one)
    #[arg(long, global = true)]
    pretty: bool,
    /// Markdown, as written
    #[arg(long, global = true)]
    md: bool,
    /// Plain text, markers stripped
    #[arg(long, global = true)]
    txt: bool,
    /// JSON, for programs
    #[arg(long, global = true)]
    json: bool,
}

impl FormatArgs {
    fn resolve(&self) -> Format {
        match (self.pretty, self.md, self.txt, self.json) {
            (true, _, _, _) => Format::Pretty,
            (_, true, _, _) => Format::Md,
            (_, _, true, _) => Format::Txt,
            (_, _, _, true) => Format::Json,
            _ => Format::default_for_stdout(),
        }
    }
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
    /// What today holds: its events, and the tasks in play
    Today,
    /// The next event, and how long there is before it
    Next,
    /// Check the connection and say what is wrong when there is something wrong
    Doctor,
    /// The token, and whether anything else is holding a copy that has gone stale
    ///
    /// There is no login here: Routine's token file is the credential and rtn
    /// re-reads it every call. This is for the clients that do not.
    Auth {
        /// Print an MCP client config carrying the token, to redirect to a file
        #[arg(long)]
        mcp_config: bool,
    },
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
    let cli = Cli::parse();
    let format = cli.format.resolve();
    match cli.command {
        Command::Log(args) => log(args, format),
        Command::Today => today_cmd(format),
        Command::Next => next_cmd(format),
        Command::Doctor => doctor(),
        Command::Auth { mcp_config } => {
            let (md, payload) = if mcp_config { auth::mcp_config()? } else { auth::status()? };
            // A config is a document, not a rendering of one -- printing it
            // through the markdown path would mangle the JSON it has to be.
            if mcp_config {
                println!("{md}");
            } else {
                render::emit(&md, &payload, format);
            }
            Ok(())
        }
    }
}

fn log(args: Log, format: Format) -> Result<(), String> {
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
        let doc: Vec<Value> = blocks.iter().map(|b| b.to_json(None)).collect();
        let mut out = format!(
            "# {}\n\nWould write {} block(s).\n\n",
            journal::title_for(day),
            blocks.len()
        );
        for b in &blocks {
            let j = b.to_json(None);
            let kind = j["type"].as_str().unwrap_or("?");
            let head = j["content"].as_str().unwrap_or("").lines().next().unwrap_or("");
            out += &format!("- `{kind}` {}\n", truncate(head, 66));
        }
        render::emit(&out, &json!({ "date": day.to_string(), "blocks": doc }), format);
        return Ok(());
    }

    let client = mcp::Client::connect().map_err(|e| e.to_string())?;
    let jrn = journal::Journal::discover(&client).map_err(|e| e.to_string())?;
    let done = journal::append(&client, &jrn, day, &blocks).map_err(|e| e.to_string())?;

    let mut line = format!("**{}** — {} block(s)", journal::title_for(day), done.written);
    if !done.tasks.is_empty() {
        line += &format!(", {} task(s)", done.tasks.len());
    }
    if done.created_day {
        line += " *(day created)*";
    }
    render::emit(
        &line,
        &json!({
            "date": day.to_string(),
            "blocks": done.written,
            "tasks": done.tasks,
            "created_day": done.created_day,
        }),
        format,
    );
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_owned();
    }
    s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
}

fn today_cmd(format: Format) -> Result<(), String> {
    let client = mcp::Client::connect().map_err(|e| e.to_string())?;
    let jrn = journal::Journal::discover(&client).map_err(|e| e.to_string())?;
    let day = today();

    let events = sorted_events(&client, day)?;
    let tasks = client
        .call("tasks_listTodaysTasks", json!({ "workspace": jrn.workspace, "limit": 50 }))
        .map_err(|e| e.to_string())?;
    let open = tasks.pointer("/todo/items").and_then(Value::as_array).cloned().unwrap_or_default();

    let mut out = format!("# {}\n\n", journal::title_for(day));
    out += "## Events\n\n";
    if events.is_empty() {
        out += "*nothing on the calendar*\n";
    }
    for e in &events {
        let start = e.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("");
        let title = collapse(e.get("title").and_then(Value::as_str).unwrap_or(""));
        out += &format!("- **{}** {}\n", start.get(11..16).unwrap_or("--:--"), truncate(&title, 66));
    }
    out += "\n## Tasks\n\n";
    if open.is_empty() {
        out += "*nothing scheduled for today*\n";
    }
    for t in &open {
        out += &format!("- [ ] {}\n", truncate(&collapse(t["title"].as_str().unwrap_or("")), 66));
    }

    render::emit(
        &out,
        &json!({ "date": day.to_string(), "events": events, "tasks": tasks }),
        format,
    );
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

fn next_cmd(format: Format) -> Result<(), String> {
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

    let (out, payload) = match next {
        None => ("*nothing else today*".to_owned(), json!({ "next": Value::Null })),
        Some(e) => {
            let start = e.pointer("/time/start_time").and_then(Value::as_str).unwrap_or("");
            let title = collapse(e.get("title").and_then(Value::as_str).unwrap_or(""));
            let mins = minutes_until(&stamp, start);
            let away = mins.map(human_gap).unwrap_or_default();
            (
                format!("**{}** {}{}", start.get(11..16).unwrap_or(""), title.trim(), away),
                json!({ "next": { "title": title.trim(), "start": start, "minutes": mins } }),
            )
        }
    };
    render::emit(&out, &payload, format);
    Ok(())
}

/// Minutes between two `YYYY-MM-DDThh:mm:ss` stamps, by the clock face. Both
/// come from the same day and the same offset, so the arithmetic stays local.
fn minutes_until(now: &str, then: &str) -> Option<i64> {
    let mins = |s: &str| -> Option<i64> {
        let h: i64 = s.get(11..13)?.parse().ok()?;
        let m: i64 = s.get(14..16)?.parse().ok()?;
        Some(h * 60 + m)
    };
    Some(mins(then)? - mins(now)?)
}

fn human_gap(mins: i64) -> String {
    match mins {
        m if m < 1 => "  *now*".to_owned(),
        m if m < 60 => format!("  *in {m} min*"),
        m if m % 60 == 0 => format!("  *in {} hr*", m / 60),
        m => format!("  *in {} hr {} min*", m / 60, m % 60),
    }
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
