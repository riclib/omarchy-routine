//! `rtn auth` — not a login, because there is nothing to log in to.
//!
//! Routine's token file is the credential, and `rtn` re-reads it on every call.
//! What this command is for is **other** clients, which do not: Claude Code
//! stores a literal `Authorization: Bearer …` header, so anything that
//! regenerates the token leaves that copy pointing at a dead one and every call
//! 401s in a way that does not look like an auth problem. Checking for that is
//! mechanical, so it should not be a thing anyone does by hand.

use crate::mcp;
use serde_json::{json, Value};
use std::fs;
use std::io::IsTerminal;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn claude_config() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude.json")
}

pub struct Registration {
    pub where_: String,
    pub stale: bool,
}

/// Every place Claude Code has Routine registered, and whether its stored
/// header still matches the live token. Both scopes are checked: user scope
/// lands at the top level, project scope under `projects.<dir>`.
fn claude_registrations(live: &str) -> Vec<Registration> {
    let raw = match fs::read_to_string(claude_config()) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(doc) => registrations_in(&doc, live),
        Err(_) => Vec::new(),
    }
}

/// Split out from the file read so the comparison can be tested without one.
pub fn registrations_in(doc: &Value, live: &str) -> Vec<Registration> {
    let mut found = Vec::new();
    let check = |node: &Value, where_: &str, out: &mut Vec<Registration>| {
        if let Some(auth) = node.pointer("/routine/headers/Authorization").and_then(Value::as_str) {
            out.push(Registration {
                where_: where_.to_owned(),
                stale: auth.trim_start_matches("Bearer ").trim() != live,
            });
        }
    };

    if let Some(servers) = doc.get("mcpServers") {
        check(servers, "user scope", &mut found);
    }
    if let Some(projects) = doc.get("projects").and_then(Value::as_object) {
        for (dir, project) in projects {
            if let Some(servers) = project.get("mcpServers") {
                check(servers, dir, &mut found);
            }
        }
    }
    found
}

/// The config another MCP client needs, ready to redirect into a file.
pub fn mcp_config() -> Result<(String, Value), String> {
    let token = mcp::token().map_err(|e| e.to_string())?;
    let doc = json!({
        "mcpServers": {
            "routine": {
                "type": "http",
                "url": "http://127.0.0.1:8765/mcp",
                "headers": { "Authorization": format!("Bearer {token}") },
            }
        }
    });
    let pretty = serde_json::to_string_pretty(&doc).unwrap();

    // The token is now wherever this went. Say so when that is a screen, but
    // do not refuse -- the whole point is that it can be redirected.
    if std::io::stdout().is_terminal() {
        eprintln!(
            "rtn: that output carries the bearer token. Redirect it to a file \
             with mode 0600 rather than leaving it in scrollback."
        );
    }
    Ok((pretty, doc))
}

pub fn status() -> Result<(String, Value), String> {
    let path = mcp::token_path();
    let token = mcp::token().map_err(|e| e.to_string())?;

    let mode = fs::metadata(&path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
    let mode_ok = mode == 0o600;

    let registrations = claude_registrations(&token);
    let stale: Vec<&Registration> = registrations.iter().filter(|r| r.stale).collect();

    let mut out = String::from("# Routine credentials\n\n");
    out += &format!("- token file `{}`\n", path.display());
    out += &format!(
        "- {} bytes, mode `{:04o}`{}\n",
        token.len(),
        mode,
        if mode_ok { "" } else { "  — **expected 0600**" }
    );
    out += "\n`rtn` re-reads this file on every call, so it has nothing of its own to go stale.\n";

    out += "\n## Other clients holding a copy\n\n";
    if registrations.is_empty() {
        out += "*none found in `~/.claude.json`*\n";
    }
    for r in &registrations {
        out += &format!(
            "- Claude Code, {} — {}\n",
            r.where_,
            if r.stale { "**stale**" } else { "current" }
        );
    }
    if !stale.is_empty() {
        out += "\nA stale copy 401s in a way that does not look like an auth problem. \
                Re-register it with the same `claude mcp add` that created it.\n";
    }

    let payload = json!({
        "token_path": path.display().to_string(),
        "mode": format!("{mode:04o}"),
        "mode_ok": mode_ok,
        "clients": registrations.iter().map(|r| json!({
            "client": "claude-code", "scope": r.where_, "stale": r.stale,
        })).collect::<Vec<_>>(),
        "any_stale": !stale.is_empty(),
    });
    Ok((out, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(user: &str, project: &str) -> Value {
        json!({
            "mcpServers": { "routine": { "headers": { "Authorization": format!("Bearer {user}") } } },
            "projects": {
                "/home/x/repo": {
                    "mcpServers": { "routine": { "headers": { "Authorization": format!("Bearer {project}") } } }
                }
            }
        })
    }

    #[test]
    fn a_matching_header_is_current() {
        let r = registrations_in(&config("live", "live"), "live");
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| !x.stale));
    }

    #[test]
    fn a_header_left_behind_by_a_rewrite_is_stale() {
        let r = registrations_in(&config("live", "old"), "live");
        let project = r.iter().find(|x| x.where_ == "/home/x/repo").unwrap();
        assert!(project.stale, "a project-scope copy must be checked too");
        assert!(!r.iter().find(|x| x.where_ == "user scope").unwrap().stale);
    }

    #[test]
    fn a_config_with_no_routine_registration_reports_none() {
        let doc = json!({ "mcpServers": { "other": {} }, "projects": {} });
        assert!(registrations_in(&doc, "live").is_empty());
    }

    #[test]
    fn the_bearer_prefix_is_not_part_of_the_comparison() {
        let doc = json!({ "mcpServers": { "routine": { "headers": { "Authorization": "Bearer  tok " } } } });
        assert!(!registrations_in(&doc, "tok")[0].stale);
    }
}
