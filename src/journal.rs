//! Today's journal entry: finding it, making it, and appending to it.

use crate::mcp::{self, Client, Error, Result};
use crate::md::{self, Block};
use serde_json::{json, Value};
use time::Date;

/// A note we will not rewrite past. A whole-document write has to name every
/// block, so an unbounded note is an unbounded request.
const MAX_BLOCKS: usize = 2000;

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August",
    "September", "October", "November", "December",
];

/// Routine titles a journal row in the app's own locale, e.g. "September 2, 2026".
/// English is the only spelling handled; a mismatch reads as "this day has no
/// entry", which would quietly create a second row for a day that already has one.
pub fn title_for(day: Date) -> String {
    format!("{} {}, {}", MONTHS[day.month() as usize - 1], day.day(), day.year())
}

pub struct Journal {
    pub workspace: String,
    pub table: String,
}

impl Journal {
    /// Resolve the workspace and the journal table rather than hardcoding either.
    pub fn discover(client: &Client) -> Result<Self> {
        let orgs = client.call("listOrganizations", json!({}))?;
        let workspace = orgs
            .pointer("/organizations/0/workspaces/0/workspaceId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Other("no workspace in listOrganizations".into()))?
            .to_owned();

        let tables = client
            .call("tables_listTables", json!({ "workspace": workspace, "limit": 50 }))?;
        let table = tables
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|t| {
                    t.get("name").and_then(Value::as_str) == Some("journal")
                })
            })
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Other("no journal table in this workspace".into()))?
            .to_owned();

        Ok(Journal { workspace, table })
    }

    fn object_id(&self, row_id: &str) -> String {
        format!(
            "object:{}:{}:{}",
            self.workspace.trim_start_matches("workspace:"),
            self.table.trim_start_matches("table:"),
            row_id.trim_start_matches("row:")
        )
    }

    /// The journal row for a day, created if that day has no entry yet.
    /// Returns the object id and whether it had to be made.
    pub fn row_for(&self, client: &Client, day: Date) -> Result<(String, bool)> {
        let title = title_for(day);
        let rows = client.call(
            "tables_searchTableRows",
            json!({
                "workspace": self.workspace, "table": self.table,
                "filter": { "match": "all", "conditions": [
                    { "field": "Title", "op": "eq", "values": [title] }
                ]},
            }),
        )?;

        if let Some(row_id) = rows.pointer("/0/0").and_then(Value::as_str) {
            return Ok((self.object_id(row_id), false));
        }

        // A field value wants the same typed envelope that reads come back in;
        // a bare string is rejected. addTableRow answers with the full
        // compound object id, so there is nothing to reassemble.
        let made = client.call(
            "tables_write_addTableRow",
            json!({
                "workspace": self.workspace, "table": self.table,
                "fields": [{ "name": "Title",
                             "value": { "type": "string", "value": title } }],
            }),
        )?;
        let id = made
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Other("addTableRow returned no id".into()))?;
        Ok((id.to_owned(), true))
    }
}

/// One block already in the note: enough to reference it and to spot the
/// trailing blank, and nothing more. Caching the content would mean holding a
/// whole day's note to append one line to it.
pub struct Existing {
    pub id: String,
    pub kind: String,
    pub blank: bool,
}

pub fn block_ids(client: &Client, row: &str) -> Result<Vec<Existing>> {
    let doc = mcp::unwrap(&client.call("tables_getObject", json!({ "object": row }))?);
    // A row created moments ago has Notes unset — not an empty document.
    let blocks = match doc.pointer("/data/Notes/blocks").and_then(Value::as_array) {
        Some(b) => b,
        None => return Ok(Vec::new()),
    };
    if blocks.len() > MAX_BLOCKS {
        return Err(Error::Other(format!(
            "that note has {} blocks, past the {MAX_BLOCKS} this will rewrite",
            blocks.len()
        )));
    }
    Ok(blocks
        .iter()
        .filter_map(|b| {
            let id = b.get("id")?.as_str()?.to_owned();
            let kind = b.get("type")?.as_str()?.to_owned();
            let blank = b
                .get("content")
                .and_then(Value::as_str)
                .map(|c| c.trim().is_empty())
                .unwrap_or(false);
            Some(Existing { id, kind, blank })
        })
        .collect())
}

pub struct Appended {
    pub written: usize,
    pub tasks: Vec<String>,
    pub created_day: bool,
}

/// Append blocks to a day's note, preserving everything already in it.
pub fn append(
    client: &Client,
    journal: &Journal,
    day: Date,
    blocks: &[Block],
) -> Result<Appended> {
    if blocks.is_empty() {
        return Err(Error::Other("nothing to write".into()));
    }
    let (row, created_day) = journal.row_for(client, day)?;

    // A todo has to carry a real task id. Authoring one with a null task makes
    // Routine mint a task server-side *and* the Electron client mint another
    // when it syncs the document, with the binding going to whichever wins.
    let mut tasks = Vec::new();
    for block in blocks.iter().filter(|b| b.needs_task()) {
        let title = match block {
            Block::Todo { content, .. } => content.clone(),
            _ => unreachable!(),
        };
        let made = client.call(
            "tasks_createTask",
            json!({
                "workspace": journal.workspace, "title": title,
                "parent": { "kind": "object", "id": row },
            }),
        )?;
        tasks.push(
            made.get("taskId")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("createTask returned no taskId".into()))?
                .to_owned(),
        );
    }

    let before = block_ids(client, &row)?;
    // Routine keeps an empty paragraph last; splice in front of it so the note
    // does not end up with a blank line through the middle of it.
    let trailing = matches!(before.last(), Some(b) if b.kind == "paragraph" && b.blank);
    let keep = before.len() - usize::from(trailing);

    let mut doc: Vec<Value> = before[..keep].iter().map(|b| md::existing(&b.id)).collect();
    let mut minted = tasks.iter();
    for block in blocks {
        doc.push(block.to_json(if block.needs_task() {
            minted.next().map(String::as_str)
        } else {
            None
        }));
    }
    if trailing {
        doc.push(md::existing(&before[keep].id));
    }

    // The ordered id list is the concurrency token; there is no ETag. Read it
    // as late as possible, so the window in which someone else can append is
    // one local round trip rather than however long we took to get here.
    let now = block_ids(client, &row)?;
    if now.len() != before.len() || now.iter().zip(&before).any(|(a, b)| a.id != b.id) {
        return Err(Error::Other(
            "the note changed while this was being prepared -- nothing written, try again".into(),
        ));
    }

    client.call(
        "tables_write_updateNotesColumn",
        json!({ "object": row, "columnName": "Notes", "content": { "blocks": doc } }),
    )?;

    Ok(Appended { written: blocks.len(), tasks, created_day })
}
