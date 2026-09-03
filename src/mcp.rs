//! The Routine MCP client.
//!
//! The server is stateless: no `initialize` handshake, no session id, one POST
//! per call. Replies are SSE-framed but never streamed — a single
//! `event: message` / `data: {…}` frame — so the whole body is read and the one
//! data line parsed. See CLAUDE.md.

use serde_json::{json, Value};
use std::fmt;
use std::fs;
use std::path::PathBuf;

const URL: &str = "http://127.0.0.1:8765/mcp";

/// Everything that can go wrong reaching Routine, kept apart because each one
/// wants a different sentence from the caller.
#[derive(Debug)]
pub enum Error {
    /// No token file, or it is not readable.
    NoToken(String),
    /// Nothing listening — Routine is closed, or its MCP server is switched off.
    NotRunning,
    /// The token is not the one the app is using.
    Unauthorised,
    /// The transport worked; Routine refused the call.
    Tool(String),
    /// The transport worked; the JSON-RPC layer refused the call.
    Rpc(String),
    /// Anything else, including a reply we could not parse.
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoToken(p) => write!(
                f,
                "no Routine token at {p}\n\
                 Routine writes it when its MCP server is enabled: Settings -> MCP."
            ),
            Error::NotRunning => write!(
                f,
                "nothing answering on {URL}\n\
                 Routine has to be running, with its MCP server enabled in Settings -> MCP."
            ),
            Error::Unauthorised => write!(
                f,
                "Routine rejected the token (401)\n\
                 It was probably rewritten since; re-read it, and re-register any client \
                 holding a stale copy."
            ),
            Error::Tool(m) => write!(f, "Routine refused the call: {m}"),
            Error::Rpc(m) => write!(f, "MCP error: {m}"),
            Error::Other(m) => write!(f, "{m}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn token_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/Routine/mcp-auth.json")
}

/// The bearer token, read fresh. Never cached to disk or environment: it is
/// rewritten by the app, and a stale copy fails in a way that does not look
/// like an auth problem.
pub fn token() -> Result<String> {
    let path = token_path();
    let raw = fs::read_to_string(&path)
        .map_err(|_| Error::NoToken(path.display().to_string()))?;
    let doc: Value = serde_json::from_str(&raw)
        .map_err(|e| Error::Other(format!("{}: {e}", path.display())))?;
    doc.get("value")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Other(format!("{}: no \"value\" field", path.display())))
}

pub struct Client {
    token: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn connect() -> Result<Self> {
        Ok(Client {
            token: token()?,
            agent: ureq::Agent::new_with_defaults(),
        })
    }

    /// Every tool the server offers, as it describes them: `name`,
    /// `description` and `inputSchema` per entry. Paged by `nextCursor` in
    /// the protocol, so the pages are followed even though the 52 here fit in
    /// one.
    pub fn list_tools(&self) -> Result<Vec<Value>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self.rpc("tools/list", params)?;
            if let Some(page) = result.get("tools").and_then(Value::as_array) {
                tools.extend(page.iter().cloned());
            }
            cursor = result.get("nextCursor").and_then(Value::as_str).map(str::to_owned);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
    }

    /// One tool call. Returns the structured result.
    ///
    /// Most tools answer with `structuredContent` already parsed; `search_search`
    /// and `tables_searchTableRows` answer with a text block holding JSON
    /// instead, so both shapes are handled here rather than at every call site.
    pub fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        let result = self.rpc("tools/call", json!({ "name": name, "arguments": arguments }))?;

        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            let m = result
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .unwrap_or("no detail");
            return Err(Error::Tool(m.to_owned()));
        }

        if let Some(structured) = result.get("structuredContent") {
            return Ok(structured.clone());
        }
        // The text-block shape.
        match result.pointer("/content/0/text").and_then(Value::as_str) {
            Some(text) => serde_json::from_str(text)
                .map_err(|e| Error::Other(format!("unparseable content block: {e}"))),
            None => Ok(Value::Null),
        }
    }

    /// One JSON-RPC round trip. Returns the `result`, or the error the
    /// transport or the RPC layer raised.
    fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });

        let response = self
            .agent
            .post(URL)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .send_json(&body);

        let mut response = match response {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(401)) => return Err(Error::Unauthorised),
            Err(ureq::Error::StatusCode(c)) => {
                return Err(Error::Other(format!("Routine answered HTTP {c}")))
            }
            Err(_) => return Err(Error::NotRunning),
        };

        let raw = response
            .body_mut()
            .read_to_string()
            .map_err(|e| Error::Other(format!("could not read the reply: {e}")))?;

        let frame = raw
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .ok_or_else(|| Error::Other("no SSE frame in the reply".into()))?;
        let msg: Value = serde_json::from_str(frame)
            .map_err(|e| Error::Other(format!("unparseable reply: {e}")))?;

        if let Some(err) = msg.get("error") {
            let m = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            return Err(Error::Rpc(m.to_owned()));
        }

        msg.get("result")
            .cloned()
            .ok_or_else(|| Error::Other("reply carried no result".into()))
    }
}

/// Strip Routine's `{"type":…,"value":…}` envelope from a whole tree.
///
/// An unset field is `{"type":"null"}` with no `value` key at all — a row
/// created moments ago has its Notes in exactly that state, which is why this
/// maps it to `null` rather than leaving the envelope in place.
pub fn unwrap(node: &Value) -> Value {
    match node {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("null") {
                return Value::Null;
            }
            if map.contains_key("type") && map.contains_key("value") {
                return unwrap(&map["value"]);
            }
            Value::Object(map.iter().map(|(k, v)| (k.clone(), unwrap(v))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(unwrap).collect()),
        other => other.clone(),
    }
}
