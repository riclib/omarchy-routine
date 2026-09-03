//! `rtn ask` — a question about your Routine, answered by a model over one
//! direct API call.
//!
//! The model is reached over HTTPS with a key and a model name the user
//! supplies in `~/.config/rtn/ask.yaml`, on either the Anthropic or the OpenAI
//! wire shape — which between them cover the vendors and every gateway and
//! local runner that imitates one. There is no coding-agent harness in the
//! path: the tools the model may call are Routine's own, fetched from the MCP
//! server and filtered here, and the loop that runs them is thirty lines with
//! a turn cap, a byte cap and a clock on it.
//!
//! **The tool list is an allowlist, and it is the security boundary.** This is
//! a model reachable from a bar overlay, acting on real data, so it can read
//! anything and create or amend a task — and it cannot delete, cannot alter a
//! table's shape, cannot touch other workspaces, and cannot send a notice to
//! another person. The list is enforced twice: the model is only *told* about
//! allowed tools, and a call is checked again at the point of execution,
//! because a model can name a tool it was never given.

use crate::mcp;
use llm_wires::{ChatRequest, Message, Provider, Tool, ToolCall, Usage, Wire};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use wire_secret::Secret;
use zeroize::Zeroizing;

/// Model calls per question. Today's dashboard is primed into the prompt, so
/// most questions take one; a write takes two (call the tool, then confirm).
/// Four leaves room for a lookup before a write and is still a bound a bar
/// overlay can live behind.
pub const MAX_TURNS: usize = 4;
/// A tool result larger than this is cut before it reaches the model. The
/// list tools are indexes and the search tool is bounded by `limit`, so this
/// is a guard against a runaway rather than a size anything sane produces.
const MAX_RESULT_BYTES: usize = 16 * 1024;
/// The whole question, tools included. Past this the overlay has long since
/// stopped being useful and the cost is only going up.
const WALL_CLOCK: Duration = Duration::from_secs(60);
/// Answer length. A short paragraph is asked for; this is the hard ceiling.
const MAX_TOKENS: u32 = 1024;

/// Everything the model may call, by the name Routine's MCP server gives it.
/// Read tools are broad; writes are the two a person would ask for out loud,
/// and nothing that destroys or notifies.
const ALLOWED: &[&str] = &[
    // read
    "search_search",
    "tasks_listTasks",
    "tasks_listTodaysTasks",
    "tasks_listUnplannedTasks",
    "tasks_getTask",
    "personal_events_listEventsForDateRange",
    "personal_events_getEvent",
    "personal_events_findAvailableTimeSlots",
    "personal_contacts_searchContacts",
    "personal_contacts_listContacts",
    "personal_pages_listPages",
    "personal_pages_getPage",
    "personal_calendars_listCalendars",
    "tables_listTables",
    "tables_getTableSchema",
    "tables_searchTableRows",
    "tables_getObject",
    "buildLink",
    // write — deliberately only these two
    "tasks_createTask",
    "tasks_updateTask",
];

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// What a config file looks like, as shown when there is none.
pub const EXAMPLE: &str = "\
provider: anthropic            # or openai — and anything speaking either shape
model: claude-haiku-4-5
# base_url: https://api.anthropic.com   # the provider's own, unless a gateway
# key: env:ANTHROPIC_API_KEY            # or the key itself, with the file at 0600
";

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/rtn/ask.yaml")
}

/// The two wire shapes. `provider:` in the file, because that is the word a
/// person reaches for; what it actually selects is which HTTP shape to speak,
/// and a gateway or a local runner picks whichever it imitates.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Anthropic,
    Openai,
}

impl Kind {
    fn base_url(self) -> &'static str {
        match self {
            Kind::Anthropic => "https://api.anthropic.com",
            Kind::Openai => "https://api.openai.com/v1",
        }
    }

    /// The variable the vendor's own tooling reads, used when `key:` is absent.
    fn conventional_var(self) -> &'static str {
        match self {
            Kind::Anthropic => "ANTHROPIC_API_KEY",
            Kind::Openai => "OPENAI_API_KEY",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Anthropic => "anthropic",
            Kind::Openai => "openai",
        }
    }
}

/// Where the key comes from. A literal is a [`Secret`] from the moment it is
/// parsed, so the file's own contents are the only place it is ever readable.
#[derive(Debug)]
enum KeySource {
    /// `key: env:NAME` — read the named variable at call time.
    Env(String),
    /// `key: sk-…` — the key itself, in the file.
    Literal(Secret),
}

impl<'de> Deserialize<'de> for KeySource {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // The scalar has to pass through a String; it is zeroed on the way.
        let raw = Zeroizing::new(String::deserialize(d)?);
        Ok(match raw.strip_prefix("env:") {
            Some(name) => KeySource::Env(name.trim().to_owned()),
            None => KeySource::Literal(Secret::from(raw.trim())),
        })
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct File {
    provider: Kind,
    model: String,
    base_url: Option<String>,
    key: Option<KeySource>,
}

#[derive(Debug)]
pub struct Config {
    kind: Kind,
    model: String,
    base_url: String,
    key: Option<KeySource>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let file: File = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
        if file.model.trim().is_empty() {
            return Err("model is empty".into());
        }
        Ok(Config {
            kind: file.provider,
            model: file.model.trim().to_owned(),
            base_url: file
                .base_url
                .map(|u| u.trim().trim_end_matches('/').to_owned())
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| file.provider.base_url().to_owned()),
            key: file.key,
        })
    }

    /// The file, or a message that includes what to write in it.
    pub fn load() -> Result<Config, String> {
        let path = config_path();
        let text = std::fs::read_to_string(&path).map_err(|_| {
            format!(
                "rtn ask needs a model and a key. Write {}:\n\n{EXAMPLE}",
                path.display()
            )
        })?;
        let config =
            Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        // A key in the file makes the file the credential, and a credential
        // readable by other users on the box is refused rather than used —
        // the same rule ssh applies to a private key.
        #[cfg(unix)]
        if matches!(config.key, Some(KeySource::Literal(_))) {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0);
            if mode & 0o077 != 0 {
                return Err(format!(
                    "{} holds a key and is readable by others (mode {mode:03o}); \
                     chmod 600 it, or use `key: env:NAME` instead",
                    path.display()
                ));
            }
        }
        Ok(config)
    }

    /// The key, resolved now rather than at load: an `env:` reference is read
    /// when the call is made, so a variable exported after the shell started
    /// counts and one unset since does not.
    pub fn key(&self) -> Result<Secret, String> {
        let from_env = |name: &str| -> Result<Secret, String> {
            match std::env::var_os(name) {
                Some(v) => Ok(Secret::new(Zeroizing::new(v.into_encoded_bytes()).to_vec())),
                None => Err(format!(
                    "the {} key is not set: export {name}, or put `key:` in {}",
                    self.kind.name(),
                    config_path().display()
                )),
            }
        };
        match &self.key {
            Some(KeySource::Literal(s)) => Ok(s.clone()),
            Some(KeySource::Env(name)) => from_env(name),
            None => from_env(self.kind.conventional_var()),
        }
    }

    pub fn wire(&self, model: Option<&str>) -> Wire {
        let model = model.unwrap_or(&self.model);
        match self.kind {
            Kind::Anthropic => Wire::anthropic(&self.base_url, model),
            Kind::Openai => Wire::openai(&self.base_url, model),
        }
    }

    /// One line for `rtn doctor`. Never the key.
    pub fn describe(&self) -> String {
        let key = match &self.key {
            Some(KeySource::Literal(_)) => "key in the file".to_owned(),
            Some(KeySource::Env(name)) => key_from_env(name),
            None => key_from_env(self.kind.conventional_var()),
        };
        format!("{} {} at {}, {key}", self.kind.name(), self.model, self.base_url)
    }
}

fn key_from_env(name: &str) -> String {
    let state = if std::env::var_os(name).is_some() { "set" } else { "NOT SET" };
    format!("key from ${name} ({state})")
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

/// The allowed tools, in the shape the wire wants, from what Routine offers.
/// The schemas are Routine's own `inputSchema`, passed through untouched —
/// they are already on the wire and hand-copying them is how they go stale.
///
/// Also returns any allowed name Routine did not offer, so a renamed tool
/// shows up in `rtn doctor` rather than as a model quietly unable to do
/// something it used to.
pub fn tools(listed: &[Value]) -> (Vec<Tool>, Vec<&'static str>) {
    let mut tools = Vec::new();
    for entry in listed {
        let Some(name) = entry.get("name").and_then(Value::as_str) else { continue };
        if !ALLOWED.contains(&name) {
            continue;
        }
        tools.push(Tool {
            name: name.to_owned(),
            description: entry
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            parameters: entry.get("inputSchema").cloned().unwrap_or(Value::Null),
        });
    }
    let missing = ALLOWED
        .iter()
        .copied()
        .filter(|a| !tools.iter().any(|t| t.name == *a))
        .collect();
    (tools, missing)
}

/// The gate every call passes before it reaches Routine: is it allowed, and
/// are its arguments JSON. Separate from executing so it can be tested
/// without a server, and so the refusal is a fact of this module rather than
/// of whatever list the model was shown.
fn gate(call: &ToolCall) -> Result<Value, String> {
    if !ALLOWED.contains(&call.name.as_str()) {
        return Err(format!("rtn does not allow {}", call.name));
    }
    if call.arguments.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&call.arguments)
        .map_err(|e| format!("the arguments for {} were not valid JSON: {e}", call.name))
}

/// A tool result the model will be shown, bounded.
fn clamp(mut s: String) -> String {
    if s.len() <= MAX_RESULT_BYTES {
        return s;
    }
    let mut cut = MAX_RESULT_BYTES;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
    s.push_str(&format!(" …[truncated by rtn at {} kB]", MAX_RESULT_BYTES / 1024));
    s
}

/// Run one call against Routine and say what happened, as text for the
/// model. An error is a result too: the model asked for something it cannot
/// have, and it should tell the user so rather than the question dying.
fn execute(client: &mcp::Client, call: &ToolCall) -> String {
    let args = match gate(call) {
        Ok(a) => a,
        Err(e) => return json!({ "error": e }).to_string(),
    };
    match client.call(&call.name, args) {
        Ok(v) => clamp(v.to_string()),
        Err(e) => json!({ "error": e.to_string() }).to_string(),
    }
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Outcome {
    pub answer: String,
    pub turns: usize,
    pub usage: Usage,
    /// Tool names, in the order they were called.
    pub calls: Vec<String>,
}

/// Send, run whatever tools come back, send again — at most [`MAX_TURNS`]
/// times. Generic over the executor so the loop is tested against a scripted
/// provider and a closure rather than a socket and a server.
async fn converse(
    provider: &dyn Provider,
    mut req: ChatRequest,
    execute: &mut dyn FnMut(&ToolCall) -> String,
) -> Result<Outcome, String> {
    let mut usage = Usage::default();
    let mut calls = Vec::new();
    for turn in 1..=MAX_TURNS {
        let resp = provider.chat(req.clone()).await.map_err(|e| e.to_string())?;
        usage.input += resp.usage.input;
        usage.output += resp.usage.output;
        usage.cached += resp.usage.cached;
        usage.cache_creation += resp.usage.cache_creation;
        req.messages.push(resp.message.clone());
        if resp.tool_calls.is_empty() {
            return Ok(Outcome {
                answer: resp.message.content.trim().to_owned(),
                turns: turn,
                usage,
                calls,
            });
        }
        for call in &resp.tool_calls {
            calls.push(call.name.clone());
            let result = execute(call);
            req.messages.push(Message::tool(call.id.clone(), result));
        }
    }
    Err(format!(
        "no answer after {MAX_TURNS} turns ({} tool calls: {})",
        calls.len(),
        calls.join(", ")
    ))
}

fn system_prompt(now: &str, workspace: &str, context: &str) -> String {
    // The field notes, as a briefing. Everything here cost time to find, and a
    // model that does not know it makes exactly the mistakes we already made.
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
titles and counts rather than describing them. Answer in plain prose, not \
markdown headings or tables.\n\
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

pub fn run(question: &str, model: Option<&str>) -> Result<(String, Value), String> {
    if question.trim().is_empty() {
        return Err("ask what?".into());
    }
    let config = Config::load()?;
    let key = config.key()?;
    let client = mcp::Client::connect().map_err(|e| e.to_string())?;
    let (offered, _missing) = tools(&client.list_tools().map_err(|e| e.to_string())?);
    let workspace = crate::journal::Journal::discover(&client)
        .map(|j| j.workspace)
        .unwrap_or_else(|_| "the personal workspace".into());
    // Priming the prompt with the day costs 10ms and saves the model several
    // round trips, which are the whole latency. Most questions asked of a bar
    // overlay are about today and need no tool call at all.
    let (now, context) = crate::ask_context();

    let req = ChatRequest {
        messages: vec![Message::user(question)],
        tools: offered,
        system: system_prompt(&now, &workspace, &context),
        max_tokens: Some(MAX_TOKENS),
        cache_stable_prefix: true,
        ..ChatRequest::default()
    };

    // Routine's client is synchronous and answers from local memory in
    // milliseconds, so it is called inline on the runtime thread; there is
    // nothing to gain from making it async for a 5ms call.
    let mut execute = |call: &ToolCall| execute(&client, call);
    let started = Instant::now();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start a runtime: {e}"))?;
    // The HTTP client wants the reactor to exist when it is built, so the
    // provider is made inside the runtime rather than handed in.
    let (info, outcome) = runtime.block_on(async {
        let provider =
            llm_wires::build(config.wire(model), Some(key)).map_err(|e| e.to_string())?;
        let outcome = tokio::time::timeout(WALL_CLOCK, converse(&*provider, req, &mut execute))
            .await
            .map_err(|_| format!("no answer within {}s", WALL_CLOCK.as_secs()))??;
        Ok::<_, String>((provider.info(), outcome))
    })?;

    let payload = json!({
        "question": question,
        "answer": outcome.answer,
        "provider": info.wire,
        "model": info.model,
        "turns": outcome.turns,
        "tools": outcome.calls,
        "duration_ms": started.elapsed().as_millis() as u64,
        "usage": {
            "input": outcome.usage.input,
            "output": outcome.usage.output,
            "cached": outcome.usage.cached,
            "cache_creation": outcome.usage.cache_creation,
        },
    });
    Ok((outcome.answer, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_wires::{ChatResponse, ChunkStream, Finish, Info};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    // -- config --

    #[test]
    fn a_full_config_parses_and_the_key_never_prints() {
        let c = Config::parse(
            "provider: openai\nmodel: gpt-4o-mini\nbase_url: https://api.x.ai/v1/\nkey: xai-secret-123\n",
        )
        .unwrap();
        assert_eq!(c.kind, Kind::Openai);
        assert_eq!(c.model, "gpt-4o-mini");
        assert_eq!(c.base_url, "https://api.x.ai/v1", "trailing slash dropped");
        let shown = format!("{c:?}");
        assert!(shown.contains("<secret>"), "{shown}");
        assert!(!shown.contains("xai-secret"), "{shown}");
        assert!(!c.describe().contains("xai-secret"));
        assert_eq!(c.key().unwrap().expose_str().unwrap(), "xai-secret-123");
    }

    #[test]
    fn defaults_fill_in_the_base_url_and_the_conventional_variable() {
        let c = Config::parse("provider: anthropic\nmodel: claude-haiku-4-5\n").unwrap();
        assert_eq!(c.base_url, "https://api.anthropic.com");
        assert!(c.describe().contains("$ANTHROPIC_API_KEY"), "{}", c.describe());
        assert_eq!(c.wire(None), Wire::anthropic("https://api.anthropic.com", "claude-haiku-4-5"));
        assert_eq!(
            c.wire(Some("claude-sonnet-4-5")),
            Wire::anthropic("https://api.anthropic.com", "claude-sonnet-4-5"),
            "--model overrides the file"
        );
    }

    #[test]
    fn an_env_key_is_read_at_call_time_and_named_when_missing() {
        let c = Config::parse("provider: openai\nmodel: m\nkey: env:RTN_TEST_KEY_A\n").unwrap();
        let err = c.key().unwrap_err();
        assert!(err.contains("RTN_TEST_KEY_A"), "{err}");
        // SAFETY: single-threaded test process, and the variable is unique to it.
        unsafe { std::env::set_var("RTN_TEST_KEY_A", "from-env") };
        assert_eq!(c.key().unwrap().expose_str().unwrap(), "from-env");
        assert!(c.describe().contains("$RTN_TEST_KEY_A (set)"), "{}", c.describe());
    }

    #[test]
    fn a_bad_config_says_what_is_wrong() {
        let cases = [
            ("provider: grok\nmodel: m\n", "grok"),
            ("provider: openai\n", "model"),
            ("provider: openai\nmodel: ''\n", "model is empty"),
            ("provider: openai\nmodel: m\napikey: x\n", "apikey"),
        ];
        for (text, want) in cases {
            let err = Config::parse(text).unwrap_err();
            assert!(err.contains(want), "{text:?} -> {err}");
        }
    }

    #[test]
    fn the_example_config_is_itself_valid() {
        let c = Config::parse(EXAMPLE).unwrap();
        assert_eq!(c.kind, Kind::Anthropic);
        assert_eq!(c.model, "claude-haiku-4-5");
    }

    // -- tools --

    #[test]
    fn only_allowed_tools_reach_the_model_and_missing_ones_are_reported() {
        let listed = vec![
            json!({"name": "tasks_getTask", "description": "One task", "inputSchema": {"type": "object"}}),
            json!({"name": "tasks_deleteTask", "description": "Gone", "inputSchema": {"type": "object"}}),
            json!({"name": "notices_createNotice", "inputSchema": {}}),
        ];
        let (tools, missing) = tools(&listed);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "tasks_getTask");
        assert_eq!(tools[0].parameters, json!({"type": "object"}));
        assert_eq!(missing.len(), ALLOWED.len() - 1);
        assert!(!missing.contains(&"tasks_getTask"));
        assert!(missing.contains(&"tasks_createTask"));
    }

    #[test]
    fn the_gate_refuses_what_the_list_omits_and_what_is_not_json() {
        let call = |name: &str, args: &str| ToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments: args.into(),
        };
        for denied in ["tasks_deleteTask", "notices_createNotice", "tables_alter_createTable", "workspaces_createWorkspace"] {
            let err = gate(&call(denied, "{}")).unwrap_err();
            assert!(err.contains("does not allow"), "{denied}: {err}");
        }
        assert_eq!(gate(&call("tasks_getTask", "")).unwrap(), json!({}));
        assert_eq!(gate(&call("tasks_getTask", r#"{"task":"task:1"}"#)).unwrap(), json!({"task": "task:1"}));
        let err = gate(&call("tasks_getTask", "{not json")).unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");
    }

    #[test]
    fn a_result_is_cut_on_a_character_boundary() {
        let long = "é".repeat(MAX_RESULT_BYTES); // two bytes each
        let cut = clamp(long);
        assert!(cut.len() < MAX_RESULT_BYTES + 64);
        assert!(cut.ends_with("kB]"), "{}", &cut[cut.len() - 40..]);
        assert_eq!(clamp("short".into()), "short");
    }

    // -- the loop --

    /// A provider that answers from a script and remembers what it was asked.
    struct Scripted {
        replies: Mutex<VecDeque<ChatResponse>>,
        seen: Mutex<Vec<ChatRequest>>,
    }

    impl Scripted {
        fn new(replies: Vec<ChatResponse>) -> Scripted {
            Scripted {
                replies: Mutex::new(replies.into()),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for Scripted {
        async fn chat(&self, req: ChatRequest) -> llm_wires::Result<ChatResponse> {
            self.seen.lock().unwrap().push(req);
            Ok(self.replies.lock().unwrap().pop_front().expect("a scripted reply"))
        }
        async fn chat_stream(&self, _: ChatRequest) -> llm_wires::Result<ChunkStream> {
            unreachable!("rtn ask does not stream")
        }
        fn info(&self) -> Info {
            Info { wire: "scripted", model: "m".into(), endpoint: "x".into() }
        }
    }

    fn says(text: &str) -> ChatResponse {
        ChatResponse {
            message: Message::assistant(text),
            tool_calls: vec![],
            usage: Usage { input: 10, output: 5, ..Usage::default() },
            finish: Some(Finish::Stop),
        }
    }

    fn asks_for(name: &str, args: &str) -> ChatResponse {
        let call = ToolCall { id: format!("call-{name}"), name: name.into(), arguments: args.into() };
        ChatResponse {
            message: Message::assistant("").with_tool_calls(vec![call.clone()]),
            tool_calls: vec![call],
            usage: Usage { input: 10, output: 5, ..Usage::default() },
            finish: Some(Finish::ToolCalls),
        }
    }

    fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(f)
    }

    #[test]
    fn a_primed_question_is_one_turn_and_no_tools() {
        let p = Scripted::new(vec![says("  Nothing until 16:00.  ")]);
        let mut ran = Vec::new();
        let out = block_on(converse(&p, ChatRequest::ask("s", "q"), &mut |c| {
            ran.push(c.name.clone());
            String::new()
        }))
        .unwrap();
        assert_eq!(out.answer, "Nothing until 16:00.");
        assert_eq!(out.turns, 1);
        assert!(ran.is_empty());
        assert!(out.calls.is_empty());
    }

    #[test]
    fn a_tool_call_is_run_and_its_result_goes_back_under_its_id() {
        let p = Scripted::new(vec![
            asks_for("tasks_getTask", r#"{"task":"task:1"}"#),
            says("It is due Friday."),
        ]);
        let out = block_on(converse(&p, ChatRequest::ask("s", "q"), &mut |c| {
            format!("ran {} with {}", c.name, c.arguments)
        }))
        .unwrap();
        assert_eq!(out.answer, "It is due Friday.");
        assert_eq!(out.turns, 2);
        assert_eq!(out.calls, vec!["tasks_getTask"]);
        assert_eq!(out.usage.input, 20, "usage is summed over turns");

        let seen = p.seen.lock().unwrap();
        let second = &seen[1].messages;
        assert_eq!(second.len(), 3, "user, assistant(tool call), tool result");
        assert_eq!(second[2].tool_call_id.as_deref(), Some("call-tasks_getTask"));
        assert_eq!(second[2].content, r#"ran tasks_getTask with {"task":"task:1"}"#);
        assert_eq!(seen[1].tools, seen[0].tools, "tools are offered on every turn");
    }

    #[test]
    fn a_model_that_never_answers_is_stopped_at_the_turn_cap() {
        let p = Scripted::new(
            (0..MAX_TURNS + 2).map(|_| asks_for("tasks_listTasks", "{}")).collect(),
        );
        let mut n = 0;
        let err = block_on(converse(&p, ChatRequest::ask("s", "q"), &mut |_| {
            n += 1;
            "[]".into()
        }))
        .unwrap_err();
        assert_eq!(n, MAX_TURNS, "one execution per turn, then stop");
        assert!(err.contains(&format!("{MAX_TURNS} turns")), "{err}");
        assert_eq!(p.seen.lock().unwrap().len(), MAX_TURNS);
    }
}
