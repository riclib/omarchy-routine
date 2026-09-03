//! A conversation, for as long as the overlay is open.
//!
//! The transcript is `rtn`'s and not the shell's: the overlay mints an id when
//! it opens, passes it on every ask, and says `--end` when it closes. QML never
//! sees a message shape or a tool call id, which keeps the rule that the shell
//! holds no API state. The file lives under the user's runtime directory —
//! private tmpfs, wiped at logout — because a transcript carries tool results,
//! which are real data.
//!
//! **Trimming is by whole exchanges.** An exchange is a question and everything
//! up to its answer, tool calls and results included. A tool call and its
//! result must stay paired or the API refuses the request, so nothing here
//! ever drops a single message.

use llm_wires::{Message, Role, ToolCall};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// A session nobody has spoken to for this long is over, whatever the
/// overlay thinks. Covers a crash that never sent `--end`.
const MAX_IDLE_SECS: u64 = 60 * 60;
/// Exchanges kept. Eight questions back is further than a bar overlay's
/// conversation goes; past it the oldest are dropped, whole.
const MAX_EXCHANGES: usize = 8;
/// The transcript on disk, and so roughly on the wire each turn. Tool
/// results are bounded at 16 kB each, so a few of them add up; oldest
/// exchanges go first until this fits.
const MAX_BYTES: usize = 96 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct StoredCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Stored {
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<StoredCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl From<&Message> for Stored {
    fn from(m: &Message) -> Stored {
        Stored {
            role: m.role.as_str().to_owned(),
            content: m.content.clone(),
            tool_calls: m
                .tool_calls
                .iter()
                .map(|c| StoredCall { id: c.id.clone(), name: c.name.clone(), arguments: c.arguments.clone() })
                .collect(),
            tool_call_id: m.tool_call_id.clone(),
        }
    }
}

impl Stored {
    fn message(&self) -> Message {
        let role = match self.role.as_str() {
            "system" => Role::System,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            _ => Role::User,
        };
        Message {
            role,
            content: self.content.clone(),
            tool_calls: self
                .tool_calls
                .iter()
                .map(|c| ToolCall { id: c.id.clone(), name: c.name.clone(), arguments: c.arguments.clone() })
                .collect(),
            tool_call_id: self.tool_call_id.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
struct Exchange {
    messages: Vec<Stored>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    /// Unix seconds of the last write; the idle clock.
    touched: u64,
    exchanges: Vec<Exchange>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Transcript {
    /// The transcript's messages, in order, ready to go ahead of a question.
    pub fn messages(&self) -> Vec<Message> {
        self.exchanges.iter().flat_map(|e| e.messages.iter().map(Stored::message)).collect()
    }

    pub fn len(&self) -> usize {
        self.exchanges.len()
    }

    /// One more question and its answer, then trim to the bounds.
    pub fn push(&mut self, messages: &[Message]) {
        self.exchanges.push(Exchange { messages: messages.iter().map(Stored::from).collect() });
        while self.exchanges.len() > MAX_EXCHANGES {
            self.exchanges.remove(0);
        }
        while self.exchanges.len() > 1 && self.bytes() > MAX_BYTES {
            self.exchanges.remove(0);
        }
        self.touched = now();
    }

    fn bytes(&self) -> usize {
        serde_json::to_vec(self).map(|v| v.len()).unwrap_or(0)
    }

    /// Parse what was on disk, or start over if it has gone idle. An
    /// unreadable file is also a fresh start: a transcript is a convenience,
    /// never the thing that stops a question being answered.
    fn revive(text: &str, at: u64) -> Transcript {
        match serde_json::from_str::<Transcript>(text) {
            Ok(t) if at.saturating_sub(t.touched) <= MAX_IDLE_SECS => t,
            _ => Transcript::default(),
        }
    }
}

/// Session ids come from the overlay as a command-line argument and become a
/// file name, so they are checked rather than trusted.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => PathBuf::from(d).join("rtn"),
        // No runtime dir means no session bus, which is not a desktop; still
        // usable from a terminal, in a private directory of our own.
        None => std::env::temp_dir().join(format!("rtn-{}", unsafe_uid())),
    }
}

fn unsafe_uid() -> String {
    // Enough to keep two users' fallback directories apart; the 0700 below
    // is what actually keeps them private.
    std::env::var("USER").unwrap_or_else(|_| "user".into())
}

fn path(id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err(format!("{id:?} is not a session id (letters, digits, - and _ only)"));
    }
    Ok(dir().join(format!("ask-{id}.json")))
}

pub fn load(id: &str) -> Result<Transcript, String> {
    let path = path(id)?;
    Ok(match std::fs::read_to_string(&path) {
        Ok(text) => Transcript::revive(&text, now()),
        Err(_) => Transcript::default(),
    })
}

pub fn save(id: &str, transcript: &Transcript) -> Result<(), String> {
    let path = path(id)?;
    let dir = path.parent().expect("a session file has a directory");
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let text = serde_json::to_string(transcript).map_err(|e| e.to_string())?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    opts.open(&path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// The overlay closed. Nothing is kept.
pub fn end(id: &str) -> Result<bool, String> {
    let path = path(id)?;
    Ok(std::fs::remove_file(path).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(q: &str, a: &str) -> Vec<Message> {
        vec![Message::user(q), Message::assistant(a)]
    }

    #[test]
    fn a_transcript_round_trips_with_its_tool_calls_paired() {
        let call = ToolCall { id: "c1".into(), name: "tasks_getTask".into(), arguments: "{}".into() };
        let messages = vec![
            Message::user("(now 13:00) when is it due?"),
            Message::assistant("").with_tool_calls(vec![call]),
            Message::tool("c1", r#"{"due":"Friday"}"#),
            Message::assistant("Friday."),
        ];
        let mut t = Transcript::default();
        t.push(&messages);
        let text = serde_json::to_string(&t).unwrap();
        let back = Transcript::revive(&text, t.touched);
        assert_eq!(back.messages(), messages);
        assert_eq!(back.len(), 1);
    }

    #[test]
    fn an_idle_session_starts_over_and_a_live_one_does_not() {
        let mut t = Transcript::default();
        t.push(&exchange("q", "a"));
        let text = serde_json::to_string(&t).unwrap();
        assert_eq!(Transcript::revive(&text, t.touched + MAX_IDLE_SECS).len(), 1);
        assert_eq!(Transcript::revive(&text, t.touched + MAX_IDLE_SECS + 1).len(), 0);
        assert_eq!(Transcript::revive("not json", 0).len(), 0, "garbage is a fresh start");
    }

    #[test]
    fn the_oldest_exchanges_go_first_and_go_whole() {
        let mut t = Transcript::default();
        for i in 0..MAX_EXCHANGES + 3 {
            t.push(&exchange(&format!("q{i}"), &format!("a{i}")));
        }
        assert_eq!(t.len(), MAX_EXCHANGES);
        let first = &t.messages()[0];
        assert_eq!(first.content, "q3", "the three oldest went");

        // A fat exchange pushes older ones out by bytes, but never itself.
        let fat = "x".repeat(MAX_BYTES);
        let mut t = Transcript::default();
        t.push(&exchange("small", "a"));
        t.push(&exchange("big", &fat));
        assert_eq!(t.len(), 1);
        assert_eq!(t.messages()[0].content, "big");
    }

    #[test]
    fn a_session_id_is_a_file_name_and_is_checked_like_one() {
        assert!(valid_id("m3k9-abc_Z"));
        for bad in ["", "../etc", "a/b", "a b", &"x".repeat(65)] {
            assert!(!valid_id(bad), "{bad:?}");
        }
        assert!(path("../x").is_err());
    }
}
