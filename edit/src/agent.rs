//! **The agent bridge** (docs/agent-panel-plan.md, A1): a turn of Claude
//! Code — `claude -p`, on the login the CLI already holds — spawned, read as
//! it streams and handed to the host as [`Event`]s. It came from the
//! modeler's `ai.rs` with the modeler taken out: what a host wants of the
//! agent (where it runs, which tools it has, what its prompt says) is a
//! [`Profile`], and nothing here knows a scene from a source file.
//!
//! **No credential passes through here.** The CLI authenticates itself; the
//! child is spawned without the API-key variables ([`subscription_only`]),
//! because a key left in the shell for something else would quietly bill an
//! API account instead of the subscription. `/login` is `claude auth login`
//! ([`auth`]), whose words are relayed and whose pasted code is forwarded.
//!
//! **Threads.** A turn runs on its own thread, which never touches a pane:
//! it queues events and asks the host to come round ([`crate::wake::wake`]).
//! The host drains them with [`poll`] on the UI thread — the standalone and
//! the modeler alike, so this file names neither event loop.
//!
//! **One process per conversation**, not per turn (measured 2026-11-17: a
//! process per turn spends 0.6–1.3 s before the model is even asked, and
//! as long again exiting; a live one starts its next turn in 10 ms). Each
//! request is a line on its stdin, each turn ends at its `result` line, and
//! Escape is an interrupt REQUEST the process answers — it stays up, and
//! the conversation with it. `--resume` is for what a live process cannot
//! carry: a restart of the editor, a setting fixed at start-up that
//! changed, a process that died. A process that will not answer an
//! interrupt is killed.
//!
//! **One agent per UI thread.** The state is a thread-local the reader
//! threads share by `Arc`, so the free functions below are what both hosts
//! call from the thread that owns their panes — and a test, which runs on a
//! thread of its own, has an agent of its own.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// What a turn tells the host, in the order it happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A few more characters of what the model is saying, as it says them.
    Delta(String),
    /// A whole message from the model: what the deltas before it added up
    /// to.
    Text(String),
    /// It called a tool: the tool's name (less the profile's prefix) and the
    /// one argument worth showing.
    Tool { name: String, arg: String },
    /// What a tool answered.
    Result(String),
    /// A tool call the CLI refused for want of permission. Its own event: a
    /// refused write nobody hears of reads as an agent that ignored the
    /// request (A0).
    Denied { tool: String, why: String },
    /// The agent wants to do something it needs leave for — run a program,
    /// touch a file outside the project — and waits for [`answer`]. ARG is
    /// the command or the path; WHY, the CLI's reason when it gives one;
    /// INPUT, the call as JSON, which the answer hands back.
    Ask { id: String, tool: String, arg: String, why: String, input: String },
    /// The agent called one of the HOST's tools ([`Profile::tools`]) and
    /// waits for [`tool_result`]. ARGS are the call's arguments by name, a
    /// value that is not text written as JSON would write it.
    Call { id: String, name: String, args: Vec<(String, String)> },
    /// A message for the host's tool server, as the CLI sent it. The bridge
    /// answers these itself and passes on only a `Call`; a host never sees
    /// one.
    Mcp { id: String, message: String },
    /// The turn's meter, one line ([`Metrics::footer`]).
    Metrics(String),
    /// The bridge's own words (a saved conversation that had gone).
    Note(String),
    /// The turn finished. SUMMARY is empty when it only repeats the last
    /// [`Event::Text`]; SESSION continues the conversation.
    Done { summary: String, session: Option<String> },
    /// The turn failed. SESSION is still the conversation's when the CLI
    /// named one — a turn that ran out of turns has not lost what it did.
    Error { text: String, session: Option<String> },
    /// [`cancel`] stopped it. Not an error: the user asked. SESSION as in
    /// `Done` — an interrupted first turn still began a conversation.
    Cancelled { session: Option<String> },
    /// A line `claude auth …` printed.
    Auth(String),
    /// `claude auth …` finished, in words.
    AuthDone(String),
    AuthError(String),
}

/// What the host says about the agent it wants. Every field is optional and
/// absent means "as the CLI would have it"; see the plan's profile table for
/// what the two hosts set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    /// The `claude` to run; else `EDIT_CLAUDE_BIN`, else the PATH's.
    pub bin: Option<PathBuf>,
    /// Where it runs: the project, so its `CLAUDE.md` loads and relative
    /// paths mean what the user means.
    pub cwd: Option<PathBuf>,
    /// A model alias or id; else `EDIT_AI_MODEL`, else [`MODEL`].
    pub model: Option<String>,
    pub max_turns: Option<String>,
    /// `--setting-sources`; `Some("")` is none at all (no hooks, no plugins).
    pub setting_sources: Option<String>,
    pub exclude_dynamic: bool,
    /// REPLACES the CLI's system prompt. By file: see [`prompt_file`].
    pub system_prompt: Option<String>,
    /// Added after the CLI's own.
    pub append_system_prompt: Option<String>,
    /// MCP servers, as the JSON `--mcp-config` takes.
    pub mcp_config: Option<String>,
    pub strict_mcp: bool,
    /// Tools that run unasked. **Not a fence**: with nobody to prompt, a
    /// tool left off this list is refused or simply runs, by the CLI's own
    /// rules. Only `denied_tools` removes one.
    pub allowed_tools: Option<String>,
    pub denied_tools: Option<String>,
    pub permission_mode: Option<String>,
    /// Who is asked when a tool needs leave. `stdio` is this bridge: the
    /// question arrives as [`Event::Ask`] and [`answer`] replies (measured
    /// 2026-11-17 — it needs no tool server). Absent, nobody is asked and
    /// such a call is refused.
    pub permission_prompt_tool: Option<String>,
    /// Stripped from a tool's name for display (`mcp__modeler__`).
    pub tool_prefix: Option<String>,
    /// **The host's own tools**, served by this bridge on the process's
    /// control channel — no second process, no socket (measured
    /// 2026-11-17): a JSON array of `{name, description, inputSchema}`. The
    /// agent sees each as `mcp__<tool_server>__<name>`; a call arrives as
    /// [`Event::Call`]. One the profile does not list in `allowed_tools`
    /// asks leave first, like any other.
    pub tools: Option<String>,
    /// The name those tools are served under; `edit` when not given.
    pub tool_server: Option<String>,
    /// The CLI's `ENABLE_TOOL_SEARCH`. With tools of the host's and nothing
    /// said here it is `false`: left to itself the CLI DEFERS MCP tools
    /// behind a search step, and the model then either spends a turn
    /// looking one up (1.5 s, measured) or calls it with arguments it
    /// guessed — `command` for a parameter named `name` (2026-11-17).
    pub tool_search: Option<String>,
    /// A directory each turn's raw stream is copied into, a file per turn.
    pub stream_log: Option<PathBuf>,
    /// The conversation to continue.
    pub session: Option<String>,
    /// Passed through after everything else.
    pub extra_args: Vec<String>,
}

impl Profile {
    /// A profile from `key value key value …`, as the Scheme side hands it
    /// over (keys by name, with or without a keyword's colon). An unknown
    /// key is an error rather than a setting silently dropped.
    pub fn from_pairs(pairs: &[String]) -> Result<Profile, String> {
        if !pairs.len().is_multiple_of(2) {
            return Err("an agent profile is KEY VALUE pairs: one key has no value".into());
        }
        let mut p = Profile::default();
        for kv in pairs.chunks(2) {
            let (key, value) = (kv[0].trim_start_matches(':'), kv[1].clone());
            let on = !matches!(value.as_str(), "" | "0" | "false" | "#f");
            match key {
                "bin" => p.bin = Some(value.into()),
                "cwd" => p.cwd = Some(value.into()),
                "model" => p.model = Some(value),
                "max-turns" => p.max_turns = Some(value),
                "setting-sources" => p.setting_sources = Some(value),
                "exclude-dynamic" => p.exclude_dynamic = on,
                "system-prompt" => p.system_prompt = Some(value),
                "append-system-prompt" => p.append_system_prompt = Some(value),
                "mcp-config" => p.mcp_config = Some(value),
                "strict-mcp" => p.strict_mcp = on,
                "allowed-tools" => p.allowed_tools = Some(value),
                "denied-tools" => p.denied_tools = Some(value),
                "permission-mode" => p.permission_mode = Some(value),
                "permission-prompt-tool" => p.permission_prompt_tool = Some(value),
                "tool-prefix" => p.tool_prefix = Some(value),
                "tools" => p.tools = Some(value),
                "tool-server" => p.tool_server = Some(value),
                "tool-search" => p.tool_search = Some(value),
                "stream-log" => p.stream_log = Some(value.into()),
                "session" => p.session = Some(value),
                "arg" => p.extra_args.push(value),
                _ => return Err(format!("unknown agent profile key: {key}")),
            }
        }
        Ok(p)
    }
}

/// Opus, by the alias, so a CLI upgrade tracks the current one without a
/// rebuild (the modeler's choice, 2026-09-22: faster on a real request than
/// the cheaper tier, and the subscription covers it).
pub const MODEL: &str = "opus";

fn model(profile: &Profile) -> String {
    profile
        .model
        .clone()
        .or_else(|| std::env::var("EDIT_AI_MODEL").ok())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| MODEL.to_string())
}

/// The `claude` this profile runs, if there is one.
pub fn claude_bin(profile: &Profile) -> Option<PathBuf> {
    let named = profile.bin.clone().or_else(|| std::env::var_os("EDIT_CLAUDE_BIN").filter(|p| !p.is_empty()).map(PathBuf::from));
    if let Some(p) = named {
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("claude")).find(|p| p.is_file())
}

const NO_CLAUDE: &str = "no `claude` on PATH — install Claude Code and log in (/login); the agent runs on your subscription";

/// The CLI runs on the SUBSCRIPTION, never on a key: it authenticates with
/// `ANTHROPIC_API_KEY` when it finds one, and the host hands its whole
/// environment to the child. That is how a modeler run meant to be covered
/// by the subscription died of "Credit balance is too low" against an API
/// account nobody meant to bill (2026-09-22).
fn subscription_only(cmd: &mut std::process::Command) {
    cmd.env_remove("ANTHROPIC_API_KEY");
    cmd.env_remove("ANTHROPIC_AUTH_TOKEN");
    cmd.env_remove("ANTHROPIC_BASE_URL");
}

/// A prompt reaches the CLI through a FILE, not argv: Linux caps one exec
/// argument at 128 KiB, and the modeler's catalogue outgrew it — every
/// request died at spawn with "Argument list too long" (2026-09-23). One
/// file per process and kind; each turn rewrites it.
fn prompt_file(kind: &str, text: &str) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!("edit-agent-{kind}-{}.md", std::process::id()));
    std::fs::write(&path, text).map_err(|e| format!("writing the {kind} prompt to {}: {e}", path.display()))?;
    Ok(path)
}

impl Profile {
    fn tool_server(&self) -> &str {
        self.tool_server.as_deref().unwrap_or("edit")
    }
}

/// The MCP servers the process is told of: the profile's own, and — when
/// the host has tools — one of type `sdk`, which the CLI reaches by asking
/// US, on its stdout, and reading the answer on its stdin.
fn mcp_config(profile: &Profile) -> Result<Option<String>, String> {
    if profile.tools.is_none() {
        return Ok(profile.mcp_config.clone());
    }
    let mut config: serde_json::Value = match &profile.mcp_config {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("the profile's mcp-config is not JSON: {e}"))?,
        None => serde_json::json!({}),
    };
    let name = profile.tool_server();
    config["mcpServers"][name] = serde_json::json!({"type": "sdk", "name": name});
    Ok(Some(config.to_string()))
}

/// How many arguments every process is started with, before the profile's.
#[cfg(test)]
const FIXED_ARGS: usize = 9;

/// The arguments a process is started with, less `--resume` — which is also
/// what makes two profiles the same process ([`send`]). Requests in and
/// events out are both stream-json, and messages arrive as they are
/// written (`--include-partial-messages`). `--bare` is never among them: it
/// also skips reading the login, so a signed-in user is told to sign in
/// (2026-09-23).
fn argv(profile: &Profile) -> Result<Vec<String>, String> {
    let mut a: Vec<String> =
        ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--model"]
            .map(String::from)
            .into();
    a.push(model(profile));
    let mut flag = |name: &str, value: &Option<String>| {
        if let Some(v) = value {
            a.push(name.to_string());
            a.push(v.clone());
        }
    };
    flag("--setting-sources", &profile.setting_sources);
    flag("--mcp-config", &mcp_config(profile)?);
    flag("--allowedTools", &profile.allowed_tools);
    flag("--disallowedTools", &profile.denied_tools);
    flag("--permission-mode", &profile.permission_mode);
    flag("--permission-prompt-tool", &profile.permission_prompt_tool);
    flag("--max-turns", &profile.max_turns);
    if profile.exclude_dynamic {
        a.push("--exclude-dynamic-system-prompt-sections".into());
    }
    if profile.strict_mcp {
        a.push("--strict-mcp-config".into());
    }
    for (name, kind, text) in [
        ("--system-prompt-file", "system", &profile.system_prompt),
        ("--append-system-prompt-file", "append", &profile.append_system_prompt),
    ] {
        if let Some(text) = text {
            a.push(name.into());
            a.push(prompt_file(kind, text)?.to_string_lossy().into_owned());
        }
    }
    a.extend(profile.extra_args.iter().cloned());
    Ok(a)
}

// ── What a turn cost ────────────────────────────────────────────────────────
// The stream carries per-message `usage` and a closing `result` with turns,
// durations and cost. The footer's TEXT is read by the modeler's harnesses
// (ai_census.py, ai_ab.py): change it and change them.

/// One turn, measured.
#[derive(Default)]
struct Metrics {
    /// Tool calls seen (the result's `num_turns` counts model turns).
    tools: usize,
    turns: u64,
    api_ms: u64,
    cost_usd: f64,
    /// The largest context any one message of the turn carried.
    max_context: u64,
    out_tokens: u64,
    cache_read: u64,
    cache_created: u64,
    /// The CLI compacted mid-turn: the model is working from a summary.
    compactions: usize,
    /// Message ids already counted. The CLI emits one `assistant` event per
    /// content block and repeats the same `usage` on each; counting them all
    /// doubled every token figure (2026-09-28).
    seen: std::collections::HashSet<String>,
}

impl Metrics {
    /// Count one `assistant` event's usage, once per message id. An event
    /// with no id always counts — better twice than not at all.
    fn saw_usage(&mut self, id: Option<&str>, u: &serde_json::Value) {
        if let Some(id) = id
            && !self.seen.insert(id.to_string())
        {
            return;
        }
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        let ctx = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
        self.max_context = self.max_context.max(ctx);
        self.out_tokens += n("output_tokens");
        self.cache_read += n("cache_read_input_tokens");
        self.cache_created += n("cache_creation_input_tokens");
    }

    fn footer(&self, wall: std::time::Duration) -> String {
        let mut s = format!(
            "{:.0}s ({:.0}s api) · {} turns · {} tools · ctx {}k · out {} tok",
            wall.as_secs_f64(),
            self.api_ms as f64 / 1000.0,
            self.turns,
            self.tools,
            self.max_context / 1000,
            self.out_tokens
        );
        // A run whose cache read is 0 turn after turn has had its prefix
        // rewritten upstream and is paying full price for it, silently.
        if self.cache_read + self.cache_created > 0 {
            s += &format!(" · cache {}k read / {}k new", self.cache_read / 1000, self.cache_created / 1000);
        }
        if self.cost_usd > 0.0 {
            s += &format!(" · ${:.2}", self.cost_usd);
        }
        if self.compactions > 0 {
            s += &format!(" · COMPACTED x{}", self.compactions);
        }
        s
    }
}

// ── Reading the stream ──────────────────────────────────────────────────────

/// What the stream said of a turn by the time it ended.
#[derive(Default)]
struct Turn {
    /// The `result` line arrived: the turn is over, whatever the pipe does.
    finished: bool,
    is_error: bool,
    /// The result's text, or its subtype when it has none (an interrupted
    /// turn's `error_during_execution`).
    summary: Option<String>,
    session: Option<String>,
    /// The model's last words, already shown: a summary that repeats them
    /// is not drawn twice.
    last_text: String,
    metrics: Metrics,
}

/// The one argument of a tool call worth a transcript row: the modeler's
/// tools name it `form`, `path` or `text`; Claude Code's own name a command,
/// a file, a pattern.
fn tool_arg(input: &serde_json::Value) -> String {
    ["form", "code", "command", "file_path", "path", "buffer", "pattern", "query", "url", "text", "prompt"]
        .iter()
        .find_map(|k| input[*k].as_str())
        .unwrap_or("")
        .to_string()
}

impl Turn {
    /// One line of stream-json. A line that is not JSON is skipped: the
    /// stream grows kinds faster than this reads them, and a cut-off last
    /// line is what a kill leaves.
    fn line(&mut self, line: &str, profile: &Profile, emit: &mut dyn FnMut(Event)) {
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) else { return };
        match ev["type"].as_str() {
            Some("assistant") => {
                self.metrics.saw_usage(ev["message"]["id"].as_str(), &ev["message"]["usage"]);
                for c in ev["message"]["content"].as_array().into_iter().flatten() {
                    match c["type"].as_str() {
                        Some("text") => {
                            if let Some(t) = c["text"].as_str() {
                                self.last_text = t.to_string();
                                emit(Event::Text(t.to_string()));
                            }
                        }
                        Some("tool_use") => {
                            self.metrics.tools += 1;
                            let name = c["name"].as_str().unwrap_or("");
                            let own = format!("mcp__{}__", profile.tool_server());
                            let name = profile.tool_prefix.as_deref().and_then(|p| name.strip_prefix(p)).unwrap_or(name);
                            let name = name.strip_prefix(&own).unwrap_or(name);
                            emit(Event::Tool { name: name.to_string(), arg: tool_arg(&c["input"]) });
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for c in ev["message"]["content"].as_array().into_iter().flatten() {
                    if c["type"].as_str() != Some("tool_result") {
                        continue;
                    }
                    let text = match &c["content"] {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Array(items) => items
                            .iter()
                            .map(|i| match i["type"].as_str() {
                                Some("image") => "[image]".to_string(),
                                _ => i["text"].as_str().unwrap_or("").to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    if !text.is_empty() {
                        emit(Event::Result(text));
                    }
                }
            }
            // A message as it is written. Only its text: the whole message
            // follows as an `assistant` event, tools and usage included.
            Some("stream_event") => {
                let e = &ev["event"];
                if e["type"].as_str() == Some("content_block_delta")
                    && e["delta"]["type"].as_str() == Some("text_delta")
                    && let Some(t) = e["delta"]["text"].as_str()
                {
                    emit(Event::Delta(t.to_string()));
                }
            }
            // The CLI asking leave for a tool call (`--permission-prompt-tool
            // stdio`). Commands it holds to be read-only, and edits a
            // permission mode already allows, never get this far.
            Some("control_request") if ev["request"]["subtype"].as_str() == Some("can_use_tool") => {
                let r = &ev["request"];
                // Named as a `Tool` event names it: the host's own tools
                // without their server's prefix.
                let tool = r["tool_name"].as_str().unwrap_or("");
                let own = format!("mcp__{}__", profile.tool_server());
                emit(Event::Ask {
                    id: ev["request_id"].as_str().unwrap_or("").to_string(),
                    tool: tool.strip_prefix(&own).unwrap_or(tool).to_string(),
                    arg: tool_arg(&r["input"]),
                    why: r["decision_reason"].as_str().unwrap_or("").to_string(),
                    input: r["input"].to_string(),
                });
            }
            // A message for the host's tool server (`type: sdk`).
            Some("control_request") if ev["request"]["subtype"].as_str() == Some("mcp_message") => emit(Event::Mcp {
                id: ev["request_id"].as_str().unwrap_or("").to_string(),
                message: ev["request"]["message"].to_string(),
            }),
            Some("system") => match ev["subtype"].as_str() {
                // The first line of every turn names the conversation, so
                // a turn cut short still knows which one it was.
                Some("init") => self.session = ev["session_id"].as_str().map(str::to_string).or(self.session.take()),
                Some("compact_boundary") => self.metrics.compactions += 1,
                Some("permission_denied") => emit(Event::Denied {
                    tool: ev["tool_name"].as_str().unwrap_or("").to_string(),
                    why: ev["message"].as_str().unwrap_or("").to_string(),
                }),
                _ => {}
            },
            Some("result") => {
                self.finished = true;
                self.is_error = ev["is_error"].as_bool().unwrap_or(false);
                self.summary = ev["result"].as_str().or(ev["subtype"].as_str()).map(str::to_string);
                self.session = ev["session_id"].as_str().map(str::to_string).or(self.session.take());
                self.metrics.turns = ev["num_turns"].as_u64().unwrap_or(0);
                self.metrics.api_ms = ev["duration_api_ms"].as_u64().unwrap_or(0);
                self.metrics.cost_usd = ev["total_cost_usd"].as_f64().unwrap_or(0.0);
            }
            _ => {}
        }
    }
}

/// Read FROM a line at a time until a turn's `result` or the end of the
/// stream — call it again for the turn after — copying each line to LOG
/// before it is parsed: a line that will not parse is the one worth having
/// on disk. Bytes, not `lines()`: one tool result that is not UTF-8 must
/// not end the turn.
fn read_stream(from: &mut impl std::io::BufRead, profile: &Profile, log: &mut Option<std::fs::File>, emit: &mut dyn FnMut(Event)) -> Turn {
    use std::io::Write;
    let mut turn = Turn::default();
    let mut bytes = Vec::new();
    while !turn.finished {
        bytes.clear();
        match from.read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&bytes);
        let line = line.trim_end_matches(['\n', '\r']);
        if let Some(f) = log.as_mut() {
            let _ = writeln!(f, "{line}");
        }
        turn.line(line, profile, emit);
    }
    turn
}

fn stream_log(profile: &Profile) -> Option<std::fs::File> {
    if std::env::var_os("EDIT_AI_NO_STREAM_LOG").is_some() {
        return None;
    }
    let dir = profile.stream_log.as_ref()?;
    std::fs::create_dir_all(dir).ok()?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    std::fs::File::create(dir.join(format!("{stamp}-{}.jsonl", std::process::id()))).ok()
}

// ── The agent's state ───────────────────────────────────────────────────────

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The `claude` a conversation is running in.
struct Proc {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    /// What it was started as: a request for anything else starts another.
    key: Vec<String>,
    /// Which process this is, so a reader that outlived its own says nothing.
    generation: u64,
    stderr: Arc<Mutex<String>>,
}

/// The turn being answered.
struct InFlight {
    bin: PathBuf,
    prompt: String,
    profile: Profile,
    began: std::time::Instant,
    /// Already started once fresh, after a saved conversation had gone.
    retried: bool,
}

struct Shared {
    events: Mutex<VecDeque<Event>>,
    proc: Mutex<Option<Proc>>,
    /// Some while a turn is being answered: what "busy" means.
    turn: Mutex<Option<InFlight>>,
    /// An interrupt was asked for and the turn has not ended yet.
    cancelling: AtomicBool,
    generation: AtomicU64,
    /// Counts turns, so a watchdog knows the turn it was set on is over.
    turns: AtomicU64,
    /// How long an interrupt is given before the process is killed, in ms.
    grace_ms: AtomicU64,
    /// Questions not yet answered: request id -> the call's input, as JSON.
    asks: Mutex<std::collections::HashMap<String, String>>,
    /// Calls of the host's tools not yet answered: request id -> the id the
    /// answer must carry, as JSON.
    calls: Mutex<std::collections::HashMap<String, String>>,
    auth_stdin: Mutex<Option<std::process::ChildStdin>>,
}

impl Shared {
    fn queue(&self, e: Event) {
        lock(&self.events).push_back(e);
        crate::wake::wake();
    }
}

thread_local! {
    static AGENT: Arc<Shared> = Arc::new(Shared {
        events: Mutex::new(VecDeque::new()),
        proc: Mutex::new(None),
        turn: Mutex::new(None),
        cancelling: AtomicBool::new(false),
        generation: AtomicU64::new(0),
        turns: AtomicU64::new(0),
        grace_ms: AtomicU64::new(3000),
        asks: Mutex::new(std::collections::HashMap::new()),
        calls: Mutex::new(std::collections::HashMap::new()),
        auth_stdin: Mutex::new(None),
    });
}

fn agent() -> Arc<Shared> {
    AGENT.with(Arc::clone)
}

/// Every event since the last call, oldest first. The host calls this when
/// it is woken, on the thread that owns its panes.
pub fn poll() -> Vec<Event> {
    lock(&agent().events).drain(..).collect()
}

/// Is a turn being answered (and so interruptible)? What a host's Escape
/// asks.
pub fn busy() -> bool {
    lock(&agent().turn).is_some()
}

// ── Starting a process, and reading it ──────────────────────────────────────

/// A request, as the line `--input-format stream-json` reads.
fn user_line(prompt: &str) -> String {
    serde_json::json!({"type": "user", "message": {"role": "user", "content": prompt}}).to_string()
}

fn write_line(stdin: &mut std::process::ChildStdin, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    writeln!(stdin, "{line}")?;
    stdin.flush()
}

/// A resume that failed on the session itself: a conversation saved in the
/// host's store whose transcript is gone from `~/.claude`.
fn stale_session(text: &str) -> bool {
    ["conversation", "session", "resume"].iter().any(|w| text.contains(w))
}

/// Start a process for TURN's profile, resuming its session when it names
/// one, and put TURN's prompt to it.
fn start(shared: &Arc<Shared>, key: Vec<String>, turn: InFlight) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(&turn.bin);
    subscription_only(&mut cmd);
    if let Some(dir) = &turn.profile.cwd {
        let _ = std::fs::create_dir_all(dir);
        cmd.current_dir(dir);
    }
    cmd.args(argv(&turn.profile)?);
    match (&turn.profile.tool_search, &turn.profile.tools) {
        (Some(v), _) => drop(cmd.env("ENABLE_TOOL_SEARCH", v)),
        (None, Some(_)) => drop(cmd.env("ENABLE_TOOL_SEARCH", "false")),
        (None, None) => {}
    }
    if let Some(id) = &turn.profile.session {
        cmd.args(["--resume", id]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", turn.bin.display()))?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return Err("the claude process has no pipes".into());
    };
    // Drain stderr on its own thread. `--verbose` (which stream-json needs)
    // chatters there; unread, the pipe's ~64 KB fills and the child BLOCKS
    // writing to it. What it said is also the error report.
    let stderr = Arc::new(Mutex::new(String::new()));
    if let Some(mut e) = child.stderr.take() {
        let buf = Arc::clone(&stderr);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = std::io::Read::read(&mut e, &mut chunk) {
                if n == 0 {
                    break;
                }
                lock(&buf).push_str(&String::from_utf8_lossy(&chunk[..n]));
            }
        });
    }
    // A process that died before it read a line says why on stderr, and the
    // reader reports that: a failed write here is not the news.
    let _ = write_line(&mut stdin, &user_line(&turn.prompt));
    let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let profile = turn.profile.clone();
    shared.turns.fetch_add(1, Ordering::SeqCst);
    *lock(&shared.turn) = Some(turn);
    *lock(&shared.proc) = Some(Proc { child, stdin, key, generation, stderr });
    let shared = Arc::clone(shared);
    std::thread::spawn(move || read_process(&shared, generation, stdout, &profile));
    Ok(())
}

/// A process's reader, for as long as it lives: turn after turn to each
/// one's `result`, then what its end means.
fn read_process(shared: &Arc<Shared>, generation: u64, stdout: std::process::ChildStdout, profile: &Profile) {
    let mut from = std::io::BufReader::new(stdout);
    let mut log = stream_log(profile);
    loop {
        let turn = read_stream(&mut from, profile, &mut log, &mut |e| match e {
            Event::Mcp { id, message } => {
                if let Some(call) = serve_tools(shared, profile, &id, &message) {
                    shared.queue(call);
                }
            }
            e => {
                if let Event::Ask { id, input, .. } = &e {
                    lock(&shared.asks).insert(id.clone(), input.clone());
                }
                shared.queue(e)
            }
        });
        // A question or a call the turn ended without hearing answered is
        // moot.
        lock(&shared.asks).clear();
        lock(&shared.calls).clear();
        if !turn.finished {
            return process_ended(shared, generation, turn);
        }
        let Some(asked) = lock(&shared.turn).take() else { continue };
        // Busy is over BEFORE the closing event is seen: whoever reads
        // `Done` may send the next request at once.
        let cancelling = shared.cancelling.swap(false, Ordering::SeqCst);
        let summary = turn.summary.unwrap_or_default();
        shared.queue(if turn.is_error && cancelling {
            Event::Cancelled { session: turn.session }
        } else if turn.is_error {
            Event::Error { text: format!("claude: {summary}"), session: turn.session }
        } else {
            shared.queue(Event::Metrics(turn.metrics.footer(asked.began.elapsed())));
            Event::Done { summary: if summary.trim() == turn.last_text.trim() { String::new() } else { summary }, session: turn.session }
        });
    }
}

/// Answer control request ID with an MCP message, on the process's stdin.
fn mcp_reply(shared: &Shared, id: &str, message: serde_json::Value) -> bool {
    let line = serde_json::json!({"type": "control_response", "response": {"subtype": "success", "request_id": id, "response": {"mcp_response": message}}});
    lock(&shared.proc).as_mut().is_some_and(|proc| write_line(&mut proc.stdin, &line.to_string()).is_ok())
}

/// The host's tool server, as much of it as needs no host: the handshake
/// and the list of tools are answered here, on the reader's thread; a CALL
/// is handed to the host as the [`Event::Call`] this returns, and answered
/// by [`tool_result`].
fn serve_tools(shared: &Shared, profile: &Profile, id: &str, message: &str) -> Option<Event> {
    let m: serde_json::Value = serde_json::from_str(message).unwrap_or(serde_json::Value::Null);
    let rpc = m["id"].clone();
    let result = |r: serde_json::Value| serde_json::json!({"jsonrpc": "2.0", "id": rpc, "result": r});
    let reply = match m["method"].as_str() {
        Some("initialize") => result(serde_json::json!({
            "protocolVersion": m["params"]["protocolVersion"].as_str().unwrap_or("2024-11-05"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": profile.tool_server(), "version": env!("CARGO_PKG_VERSION")},
        })),
        Some("tools/list") => {
            let tools: serde_json::Value = profile.tools.as_deref().and_then(|t| serde_json::from_str(t).ok()).unwrap_or(serde_json::json!([]));
            result(serde_json::json!({"tools": tools}))
        }
        Some("tools/call") => {
            let args = m["params"]["arguments"].as_object().into_iter().flatten();
            let args = args.map(|(k, v)| (k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect();
            lock(&shared.calls).insert(id.to_string(), rpc.to_string());
            return Some(Event::Call { id: id.to_string(), name: m["params"]["name"].as_str().unwrap_or("").to_string(), args });
        }
        // A notification wants no answer of its own; the control request
        // that carried it still does.
        _ if rpc.is_null() => serde_json::json!({"jsonrpc": "2.0", "id": 0, "result": {}}),
        Some(other) => serde_json::json!({"jsonrpc": "2.0", "id": rpc, "error": {"code": -32601, "message": format!("no such method: {other}")}}),
        None => return None,
    };
    mcp_reply(shared, id, reply);
    None
}

/// Answer the call [`Event::Call`] ID made: TEXT is what the tool says, and
/// FAILED marks it as an error the agent should read as one. False when the
/// call is no longer open.
pub fn tool_result(id: &str, text: &str, failed: bool) -> bool {
    let shared = agent();
    let Some(rpc) = lock(&shared.calls).remove(id) else { return false };
    let rpc: serde_json::Value = serde_json::from_str(&rpc).unwrap_or(serde_json::Value::Null);
    let result = serde_json::json!({"content": [{"type": "text", "text": text}], "isError": failed});
    mcp_reply(&shared, id, serde_json::json!({"jsonrpc": "2.0", "id": rpc, "result": result}))
}

/// The stream ended without a `result`: the process is gone. Said only by
/// the reader of the process that is still the agent's — one that was
/// killed and replaced was reported by whoever killed it.
fn process_ended(shared: &Arc<Shared>, generation: u64, cut: Turn) {
    let Some(mut proc) = ({
        let mut slot = lock(&shared.proc);
        if slot.as_ref().is_some_and(|p| p.generation == generation) { slot.take() } else { None }
    }) else {
        return;
    };
    let status = proc.child.wait();
    let Some(asked) = lock(&shared.turn).take() else { return };
    if shared.cancelling.swap(false, Ordering::SeqCst) {
        return shared.queue(Event::Cancelled { session: cut.session });
    }
    // stderr closes with the process; give its pump a moment to finish.
    let said = |buf: &Mutex<String>| lock(buf).trim().to_string();
    let mut err = said(&proc.stderr);
    for _ in 0..20 {
        if !err.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        err = said(&proc.stderr);
    }
    // A saved conversation that has gone must not wedge the pane: say so,
    // drop the id, and ask once more, fresh.
    if asked.profile.session.is_some() && !asked.retried && stale_session(&err) {
        shared.queue(Event::Note("the saved conversation is gone — starting fresh".into()));
        let mut profile = asked.profile;
        profile.session = None;
        let again = InFlight { profile, retried: true, ..asked };
        if let Err(text) = start(shared, proc.key, again) {
            shared.queue(Event::Error { text, session: None });
        }
        return;
    }
    let status = status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
    shared.queue(Event::Error { text: format!("claude exited with {status}: {err}"), session: cut.session });
}

/// Kill the process, if there is one, and close the turn it was answering
/// as cancelled.
fn kill(shared: &Shared) {
    if let Some(mut proc) = lock(&shared.proc).take() {
        let _ = proc.child.kill();
        let _ = proc.child.wait();
    }
    shared.cancelling.store(false, Ordering::SeqCst);
    if lock(&shared.turn).take().is_some() {
        shared.queue(Event::Cancelled { session: None });
    }
}

/// Put PROMPT to the agent PROFILE describes: on the process already
/// running that conversation, or a new one — resuming `profile.session` —
/// when there is none, it has died, or the profile asks for something a
/// running process cannot change (its directory, model, tools, prompt).
/// Returns at once — the caller is the UI thread — and the turn arrives
/// through [`poll`], ending in one of `Done`, `Error` or `Cancelled`.
/// Refuses while a turn is being answered, and when there is no `claude`.
pub fn send(prompt: String, profile: Profile) -> Result<(), String> {
    let shared = agent();
    let bin = claude_bin(&profile).ok_or(NO_CLAUDE)?;
    if busy() {
        return Err("the agent is still answering — Escape interrupts it".into());
    }
    let mut key = argv(&profile)?;
    key.push(bin.to_string_lossy().into_owned());
    key.push(profile.cwd.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default());
    key.push(profile.tool_prefix.clone().unwrap_or_default());
    key.push(profile.tool_search.clone().unwrap_or_default());
    shared.cancelling.store(false, Ordering::SeqCst);
    let turn = InFlight { bin, prompt, profile, began: std::time::Instant::now(), retried: false };
    {
        let mut slot = lock(&shared.proc);
        if let Some(proc) = slot.as_mut()
            && proc.key == key
            && matches!(proc.child.try_wait(), Ok(None))
            && write_line(&mut proc.stdin, &user_line(&turn.prompt)).is_ok()
        {
            shared.turns.fetch_add(1, Ordering::SeqCst);
            *lock(&shared.turn) = Some(turn);
            return Ok(());
        }
        // Another profile's process, or one that has died: gone. Its reader
        // finds the slot empty and says nothing.
        if let Some(mut old) = slot.take() {
            let _ = old.child.kill();
            let _ = old.child.wait();
        }
    }
    start(&shared, key, turn)
}

/// Stop the turn being answered; false when there was none. The process is
/// ASKED to stop, and stays up with the conversation: the turn closes as
/// [`Event::Cancelled`] when it answers. One that has not answered within
/// the grace period is killed, and the next request resumes the session in
/// a new one.
///
/// What its tools already did stays done: an interrupt stops the work, it
/// does not undo it.
pub fn cancel() -> bool {
    let shared = agent();
    if !busy() {
        return false;
    }
    shared.cancelling.store(true, Ordering::SeqCst);
    let asked = lock(&shared.proc).as_mut().is_some_and(|proc| {
        let line = serde_json::json!({"type": "control_request", "request_id": "interrupt", "request": {"subtype": "interrupt"}});
        write_line(&mut proc.stdin, &line.to_string()).is_ok()
    });
    if !asked {
        kill(&shared);
        return true;
    }
    let turn = shared.turns.load(Ordering::SeqCst);
    let grace = std::time::Duration::from_millis(shared.grace_ms.load(Ordering::SeqCst));
    std::thread::spawn(move || {
        std::thread::sleep(grace);
        if shared.cancelling.load(Ordering::SeqCst) && shared.turns.load(Ordering::SeqCst) == turn {
            kill(&shared);
        }
    });
    true
}

/// Answer the question [`Event::Ask`] ID put: ALLOW the call as it was
/// asked, or refuse it — MESSAGE is what the agent is told, so a refusal
/// can say what to do instead. False when the question is no longer open
/// (the turn ended, or it was answered).
pub fn answer(id: &str, allow: bool, message: &str) -> bool {
    let shared = agent();
    let Some(input) = lock(&shared.asks).remove(id) else { return false };
    let verdict = if allow {
        let input: serde_json::Value = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
        serde_json::json!({"behavior": "allow", "updatedInput": input})
    } else {
        serde_json::json!({"behavior": "deny", "message": if message.is_empty() { "The user said no." } else { message }})
    };
    let line = serde_json::json!({"type": "control_response", "response": {"subtype": "success", "request_id": id, "response": verdict}});
    lock(&shared.proc).as_mut().is_some_and(|proc| write_line(&mut proc.stdin, &line.to_string()).is_ok())
}

/// End the conversation's process — a NEW conversation must not be the old
/// one's next turn. A turn being answered closes as cancelled.
pub fn stop() {
    kill(&agent());
}

// ── /login and friends: `claude auth …` ─────────────────────────────────────
// `claude auth login` works headless: it opens the browser itself, prints
// the URL as a fallback, then waits on STDIN for the code the signed-in page
// shows. So the child's stdin is kept, and the next line typed in the pane
// while a login is pending goes to it ([`auth_input`]).
//
// The login is the SUBSCRIPTION's (`--claudeai`), said outright rather than
// left to the CLI's default: its other mode, `--console`, is API billing.

/// Is a login waiting for its pasted code?
pub fn auth_pending() -> bool {
    lock(&agent().auth_stdin).is_some()
}

/// Forward one line (the pasted code) to the pending login; false when none
/// is pending or it has gone.
pub fn auth_input(text: &str) -> bool {
    match lock(&agent().auth_stdin).as_mut() {
        Some(stdin) => write_line(stdin, text.trim()).is_ok(),
        None => false,
    }
}

/// Run `claude auth ACTION` (login, logout or status) on its own thread:
/// what it prints arrives as [`Event::Auth`] lines, and it closes with
/// `AuthDone` or `AuthError`.
pub fn auth(action: &str, profile: &Profile) -> Result<(), String> {
    if !["login", "logout", "status"].contains(&action) {
        return Err(format!("claude auth {action}: not one of login, logout, status"));
    }
    let bin = claude_bin(profile).ok_or(NO_CLAUDE)?;
    let (shared, action) = (agent(), action.to_string());
    std::thread::spawn(move || {
        let end = match run_auth(&shared, &bin, &action) {
            Ok(s) => Event::AuthDone(s),
            Err(e) => Event::AuthError(e),
        };
        *lock(&shared.auth_stdin) = None;
        shared.queue(end);
    });
    Ok(())
}

/// `claude auth status`'s JSON, in words — and loudly, when the login is
/// not the subscription's.
fn status_words(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json.trim()) {
        Ok(j) if j["loggedIn"].as_bool() == Some(true) => {
            let who = j["email"].as_str().unwrap_or("?");
            match j["authMethod"].as_str() {
                Some("claude.ai") | None => format!("logged in as {who} ({})", j["subscriptionType"].as_str().unwrap_or("?")),
                Some(other) => format!("logged in as {who} through {other} — NOT your Claude subscription: /logout, then /login"),
            }
        }
        Ok(_) => "not logged in — /login to sign in".into(),
        Err(_) => json.trim().to_string(),
    }
}

fn run_auth(shared: &Shared, bin: &Path, action: &str) -> Result<String, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(bin);
    subscription_only(&mut cmd);
    cmd.args(["auth", action]);
    if action == "login" {
        cmd.arg("--claudeai");
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", bin.display()))?;
    *lock(&shared.auth_stdin) = child.stdin.take();
    let mut stdout = child.stdout.take().ok_or("no stdout on the claude process")?;
    // Chunks, not lines: the login's "Paste code here if prompted >" never
    // ends in a newline, and the pane must show it anyway. `status` prints
    // JSON meant for the words below, not the pane.
    let relay = action != "status";
    let mut all = String::new();
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    let say = |line: &str, all: &mut String| {
        all.push_str(line);
        all.push('\n');
        if relay {
            shared.queue(Event::Auth(line.to_string()));
        }
    };
    loop {
        let n = stdout.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        acc.push_str(&String::from_utf8_lossy(&buf[..n]));
        while let Some(pos) = acc.find('\n') {
            let line: String = acc.drain(..=pos).collect();
            if !line.trim().is_empty() {
                say(line.trim(), &mut all);
            }
        }
        if !acc.trim().is_empty() {
            let rest = acc.trim().to_string();
            say(&rest, &mut all);
            acc.clear();
        }
    }
    let status = child.wait().map_err(|e| format!("waiting for claude: {e}"))?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        let err = if err.trim().is_empty() { all } else { err };
        return Err(format!("claude auth {action} exited with {status}: {}", err.trim()));
    }
    Ok(match action {
        "status" => status_words(&all),
        "login" => "logged in — ask away".into(),
        _ => "logged out".into(),
    })
}

/// A stand-in `claude` for tests, here and in the binding: a shell script
/// that keeps its arguments beside itself (`$D/argv`, and a line in
/// `$D/starts` each time it is started as an agent) and then runs BODY, with `$D` its directory.
#[cfg(test)]
pub(crate) fn fake_claude(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("edit-agent-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("claude");
    let script = format!("#!/bin/sh\nD='{}'\nprintf '%s\\n' \"$@\" > \"$D/argv\"\n[ \"$1\" = auth ] || echo \"started tool-search=$ENABLE_TOOL_SEARCH\" >> \"$D/starts\"\n{body}\n", dir.display());
    // Written under another name and renamed: a test on another thread that
    // spawns while this file is open for writing would hand its child the
    // descriptor, and running the script would then fail as "text busy".
    let tmp = dir.join("claude.new");
    std::fs::write(&tmp, script).unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&tmp, &bin).unwrap();
    bin
}

/// A stand-in's body that holds a conversation, as `claude` does with
/// `--input-format stream-json`: each request line is kept in `$D/stdin`
/// and answered by ANSWER (shell, run once per request, the line in
/// `$line`); a request with "long" in it is begun and left unanswered; an
/// interrupt closes the turn the way the CLI does; a request with "leave"
/// in it asks leave to run `cargo test` and closes on the answer, and one
/// with "outside" leave to Write `$D/elsewhere/notes.txt`; one with
/// "call the tool" in it shakes hands with the host's tool server, calls
/// `buffer_read` and closes on that answer, kept in `$D/tool-answer`.
#[cfg(test)]
pub(crate) fn fake_conversation(answer: &str) -> String {
    format!(
        r#"while IFS= read -r line; do
  printf '%s\n' "$line" >> "$D/stdin"
  case "$line" in
    *control_request*)
      echo '{{"type":"control_response","response":{{"subtype":"success"}}}}'
      echo '{{"type":"result","subtype":"error_during_execution","is_error":true,"result":null,"session_id":"s-1"}}';;
    *mcp_response*)
      case "$line" in
        *tools*|*protocolVersion*) ;;
        *) printf '%s\n' "$line" > "$D/tool-answer"
           echo '{{"type":"result","is_error":false,"result":"read it","session_id":"s-1","num_turns":2}}';;
      esac;;
    *control_response*)
      case "$line" in
        *allow*) echo '{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"42"}}]}}}}';;
        *) echo '{{"type":"user","message":{{"content":[{{"type":"tool_result","is_error":true,"content":"refused"}}]}}}}';;
      esac
      echo '{{"type":"result","is_error":false,"result":"so it went","session_id":"s-1","num_turns":2}}';;
    *"call the tool"*)
      echo '{{"type":"control_request","request_id":"c-0","request":{{"subtype":"mcp_message","server_name":"edit","message":{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-11-25"}}}}}}}}'
      echo '{{"type":"control_request","request_id":"c-1","request":{{"subtype":"mcp_message","server_name":"edit","message":{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}}}}}'
      echo '{{"type":"assistant","message":{{"id":"m","content":[{{"type":"tool_use","name":"mcp__edit__buffer_read","input":{{"buffer":"main.scm","from":2}}}}]}}}}'
      echo '{{"type":"control_request","request_id":"c-2","request":{{"subtype":"mcp_message","server_name":"edit","message":{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"buffer_read","arguments":{{"buffer":"main.scm","from":2}}}}}}}}}}';;
    *outside*)
      printf '{{"type":"control_request","request_id":"q-2","request":{{"subtype":"can_use_tool","tool_name":"Write","input":{{"file_path":"%s","content":"x"}}}}}}\n' "$D/elsewhere/notes.txt";;
    *leave*)
      echo '{{"type":"control_request","request_id":"q-1","request":{{"subtype":"can_use_tool","tool_name":"Bash","input":{{"command":"cargo test","description":"Run the tests"}},"decision_reason":"This command requires approval"}}}}';;
    *long*)
      echo '{{"type":"system","subtype":"init","session_id":"s-1"}}'
      echo '{{"type":"assistant","message":{{"id":"m","content":[{{"type":"text","text":"working"}}]}}}}';;
    *)
{answer}
      ;;
  esac
done"#
    )
}

/// Wait, in a test, for the agent's events until UNTIL is satisfied by all
/// of them so far.
#[cfg(test)]
pub(crate) fn wait_for(until: &dyn Fn(&[Event]) -> bool) -> Vec<Event> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut events = Vec::new();
    loop {
        events.extend(poll());
        if until(&events) {
            return events;
        }
        assert!(std::time::Instant::now() < deadline, "never came: {events:#?}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream as `claude` 2.1.303 writes one (A0), cut to the fields that
    /// are read: hooks and thinking the reader has no use for, a message
    /// arriving in pieces and then whole, split over two events with one
    /// usage, a tool call and its answer, a refusal, a compaction, the
    /// result — and the first line of the turn after.
    const STREAM: &str = r#"{"type":"system","subtype":"hook_started","hook_name":"SessionStart"}
{"type":"system","subtype":"init","cwd":"/p","permissionMode":"acceptEdits","session_id":"s-1"}
{"type":"system","subtype":"thinking_tokens","estimated_tokens":12}
{"type":"stream_event","event":{"type":"message_start"}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hm"}}}
{"type":"assistant","message":{"id":"m1","content":[{"type":"thinking","thinking":""}],"usage":{"input_tokens":4,"cache_read_input_tokens":30000,"cache_creation_input_tokens":2000,"output_tokens":50}}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"I'll fix "}}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"the typo."}}}
{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"I'll fix the typo."}],"usage":{"input_tokens":4,"cache_read_input_tokens":30000,"cache_creation_input_tokens":2000,"output_tokens":50}}}
{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/p/main.rs","old_string":"helo","new_string":"hello"}}],"usage":{"input_tokens":5,"cache_read_input_tokens":32000,"cache_creation_input_tokens":100,"output_tokens":20}}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"The file /p/main.rs has been updated."}]}}
{"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t2","name":"mcp__modeler__eval","input":{"form":"(add-sphere)"}}],"usage":{}}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"ok"},{"type":"image","source":{}}]}]}}
{"type":"assistant","message":{"id":"m4","content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"cargo test","description":"Run the tests"}}],"usage":{}}}
{"type":"system","subtype":"permission_denied","tool_name":"Bash","tool_use_id":"t3","message":"This command requires approval"}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","is_error":true,"content":"This command requires approval"}]}}
{"type":"system","subtype":"compact_boundary"}
{"type":"rate_limit_event","rate_limit_info":{}}
not json at all
{"type":"assistant","message":{"id":"m5","content":[{"type":"text","text":"Fixed; I could not run the tests."}],"usage":{"input_tokens":6,"cache_read_input_tokens":33000,"cache_creation_input_tokens":0,"output_tokens":30}}}
{"type":"result","subtype":"success","is_error":false,"result":"Fixed; I could not run the tests.","session_id":"s-1","num_turns":5,"duration_ms":9000,"duration_api_ms":7000,"total_cost_usd":0.12}
{"type":"assistant","message":{"id":"m6","content":[{"type":"text","text":"the next turn's"}]}}
"#;

    /// Read turns out of STREAM until it ends.
    fn read(stream: &str, profile: &Profile) -> Vec<(Turn, Vec<Event>)> {
        let mut from = stream.as_bytes();
        let mut turns = Vec::new();
        loop {
            let mut events = Vec::new();
            let turn = read_stream(&mut from, profile, &mut None, &mut |e| events.push(e));
            let over = !turn.finished;
            turns.push((turn, events));
            if over {
                return turns;
            }
        }
    }

    fn tool(name: &str, arg: &str) -> Event {
        Event::Tool { name: name.into(), arg: arg.into() }
    }

    #[test]
    fn a_stream_reads_as_the_turns_it_was() {
        let profile = Profile { tool_prefix: Some("mcp__modeler__".into()), ..Profile::default() };
        let mut turns = read(STREAM, &profile);
        assert_eq!(turns.len(), 2, "a result ends a turn; the stream goes on");
        let (turn, events) = turns.remove(0);
        assert_eq!(
            events,
            vec![
                Event::Delta("I'll fix ".into()),
                Event::Delta("the typo.".into()),
                Event::Text("I'll fix the typo.".into()),
                tool("Edit", "/p/main.rs"),
                Event::Result("The file /p/main.rs has been updated.".into()),
                tool("eval", "(add-sphere)"),
                Event::Result("ok\n[image]".into()),
                tool("Bash", "cargo test"),
                Event::Denied { tool: "Bash".into(), why: "This command requires approval".into() },
                Event::Result("This command requires approval".into()),
                Event::Text("Fixed; I could not run the tests.".into()),
            ]
        );
        assert!(turn.finished && !turn.is_error);
        assert_eq!(turn.session.as_deref(), Some("s-1"));
        assert_eq!(turn.summary.as_deref(), Some("Fixed; I could not run the tests."));
        assert_eq!(turn.last_text, "Fixed; I could not run the tests.", "so the summary is not drawn twice");
        let m = &turn.metrics;
        assert_eq!((m.tools, m.turns, m.compactions), (3, 5, 1));
        assert_eq!(m.out_tokens, 100, "m1's usage counted once, though it came twice");
        assert_eq!(m.max_context, 33_006, "the largest message, not the sum");
        assert_eq!(
            m.footer(std::time::Duration::from_secs(9)),
            "9s (7s api) · 5 turns · 3 tools · ctx 33k · out 100 tok · cache 95k read / 2k new · $0.12 · COMPACTED x1"
        );
        let (next, events) = turns.remove(0);
        assert_eq!(events, vec![Event::Text("the next turn's".into())]);
        assert!(!next.finished && next.metrics.tools == 0, "and starts from nothing");
    }

    #[test]
    fn a_stream_cut_short_is_not_a_finished_turn() {
        // A kill, or a crash, mid-line: what was whole is still delivered,
        // the turn knows it never closed — and which conversation it was.
        let cut = &STREAM[..STREAM.find("Fixed; I could").unwrap()];
        let (turn, events) = read(cut, &Profile::default()).remove(0);
        assert!(!turn.finished);
        assert_eq!(turn.session.as_deref(), Some("s-1"), "from the turn's first line");
        assert_eq!(events.len(), 10);
        assert_eq!(events[5], tool("mcp__modeler__eval", "(add-sphere)"), "no prefix asked for, none stripped");
        // Bytes that are not UTF-8 cost their line, not the turn.
        let mut bytes = b"{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"\xff\xfe\"}]}}\n".to_vec();
        bytes.extend_from_slice(b"{\"type\":\"result\",\"is_error\":false,\"result\":\"done\"}\n");
        let mut events = Vec::new();
        let turn = read_stream(&mut &bytes[..], &Profile::default(), &mut None, &mut |e| events.push(e));
        assert!(turn.finished);
        assert_eq!(events.len(), 1, "{events:?}");
    }

    #[test]
    fn an_interrupted_result_has_no_text_and_says_why() {
        // What `claude` answers an interrupt with (A0): result is null.
        let stream = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":null,"session_id":"s-9"}"#;
        let (turn, _) = read(stream, &Profile::default()).remove(0);
        assert!(turn.finished && turn.is_error);
        assert_eq!(turn.summary.as_deref(), Some("error_during_execution"));
        assert_eq!(turn.session.as_deref(), Some("s-9"), "the conversation is still there to resume");
    }

    #[test]
    fn a_missing_usage_field_costs_nothing() {
        let mut m = Metrics::default();
        m.saw_usage(None, &serde_json::Value::Null);
        assert_eq!(m.max_context, 0);
        assert_eq!(m.footer(std::time::Duration::from_secs(1)), "1s (0s api) · 0 turns · 0 tools · ctx 0k · out 0 tok");
    }

    fn strings(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn a_profile_is_read_from_pairs_and_becomes_the_arguments() {
        let p = Profile::from_pairs(&strings(&[
            ":cwd", "/work/proj", "model", "sonnet", "permission-mode", "acceptEdits", "denied-tools", "Bash",
            "setting-sources", "", "strict-mcp", "1", "exclude-dynamic", "#f", "session", "s-7",
            "append-system-prompt", "You are in edit.", "arg", "--add-dir", "arg", "/elsewhere",
        ]))
        .expect("a profile");
        assert_eq!(p.cwd.as_deref(), Some(Path::new("/work/proj")));
        assert_eq!(p.session.as_deref(), Some("s-7"));
        let a = argv(&p).expect("argv");
        assert_eq!(a[0], "-p");
        let after = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].as_str());
        assert_eq!(after("--input-format"), Some("stream-json"), "requests are lines on stdin");
        assert_eq!(after("--output-format"), Some("stream-json"));
        assert!(a.contains(&"--include-partial-messages".to_string()), "a reply is shown as it is written");
        assert_eq!(after("--model"), Some("sonnet"));
        assert_eq!(after("--setting-sources"), Some(""), "none at all is said, not left out");
        assert_eq!(after("--disallowedTools"), Some("Bash"));
        assert_eq!(after("--permission-mode"), Some("acceptEdits"));
        assert_eq!(after("--resume"), None, "the session is not what makes two profiles one process");
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert!(!a.contains(&"--exclude-dynamic-system-prompt-sections".to_string()));
        assert!(!a.contains(&"--bare".to_string()) && !a.contains(&"--system-prompt-file".to_string()));
        let file = after("--append-system-prompt-file").expect("the prompt goes by file");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "You are in edit.");
        assert_eq!(&a[a.len() - 2..], &strings(&["--add-dir", "/elsewhere"])[..]);

        assert!(Profile::from_pairs(&strings(&["modle", "opus"])).unwrap_err().contains("modle"), "a misspelt key is refused");
        assert!(Profile::from_pairs(&strings(&["model"])).is_err());
        assert_eq!(argv(&Profile::default()).unwrap().len(), FIXED_ARGS, "an empty profile asks for nothing more");
    }

    /// One answered request, as the stand-in prints it.
    const ANSWER: &str = r#"      echo '{"type":"system","subtype":"init","session_id":"s-1"}'
      echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}}'
      echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hello"}]}}'
      echo '{"type":"result","is_error":false,"result":"hello","session_id":"s-1","num_turns":1}'"#;

    fn ended(events: &[Event]) -> bool {
        matches!(events.last(), Some(Event::Done { .. } | Event::Error { .. } | Event::Cancelled { .. }))
    }

    fn lines(dir: &Path, file: &str) -> Vec<String> {
        std::fs::read_to_string(dir.join(file)).unwrap_or_default().lines().map(str::to_string).collect()
    }

    #[test]
    fn a_conversation_is_one_process_and_a_changed_profile_is_another() {
        let bin = fake_claude("conversation", &fake_conversation(ANSWER));
        let dir = bin.parent().unwrap().to_path_buf();
        let profile = Profile { bin: Some(bin), cwd: Some(dir.join("proj")), stream_log: Some(dir.join("streams")), ..Profile::default() };
        let mut cmd = std::process::Command::new("true");
        subscription_only(&mut cmd);
        assert!(cmd.get_envs().any(|(k, v)| k == "ANTHROPIC_API_KEY" && v.is_none()), "a key in the shell does not reach the child");

        send("say \"hello\"".into(), profile.clone()).expect("sent");
        assert!(busy());
        assert!(send("another".into(), profile.clone()).unwrap_err().contains("still answering"));
        let events = wait_for(&ended);
        assert_eq!(events[..2], [Event::Delta("hel".into()), Event::Text("hello".into())]);
        assert!(matches!(&events[2], Event::Metrics(m) if m.contains("1 turns · 0 tools")), "{events:?}");
        assert_eq!(events[3], Event::Done { summary: String::new(), session: Some("s-1".into()) }, "the summary only repeated the text");
        assert!(!busy(), "busy ended with the turn, though the process lives");

        // The next request is a line to the SAME process: no start, no
        // --resume, nothing to wait for but the model.
        send("and again".into(), Profile { session: Some("s-1".into()), ..profile.clone() }).expect("sent");
        wait_for(&ended);
        assert_eq!(lines(&dir, "starts").len(), 1, "one process for both");
        assert!(!lines(&dir, "argv").contains(&"--resume".to_string()));
        let asked: Vec<serde_json::Value> = lines(&dir, "stdin").iter().map(|l| serde_json::from_str(l).expect("a JSON line")).collect();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0]["message"]["content"], "say \"hello\"", "the prompt, quoted as JSON wants");
        assert_eq!((asked[1]["type"].as_str(), asked[1]["message"]["content"].as_str()), (Some("user"), Some("and again")));
        assert!(dir.join("proj").is_dir(), "it runs where the profile said");
        assert_eq!(std::fs::read_dir(dir.join("streams")).unwrap().count(), 1, "the raw stream is kept, a file per process");

        // A setting a running process cannot change: another process, in
        // the same conversation.
        send("now as sonnet".into(), Profile { model: Some("sonnet".into()), session: Some("s-1".into()), ..profile }).expect("sent");
        wait_for(&ended);
        assert_eq!(lines(&dir, "starts").len(), 2);
        let argv = lines(&dir, "argv").join("\n");
        assert!(argv.contains("--model\nsonnet") && argv.ends_with("--resume\ns-1"), "{argv}");
        stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_turn_says_what_the_cli_said() {
        // Dies before a result: the exit status and stderr are the report.
        let bin = fake_claude("dies", "echo 'Not logged in · Please run /login' >&2\nexit 1");
        send("hi".into(), Profile { bin: Some(bin.clone()), ..Profile::default() }).expect("sent");
        let events = wait_for(&ended);
        let [Event::Error { text, session: None }] = &events[..] else { panic!("{events:?}") };
        assert!(text.contains("Not logged in") && text.contains("exit"), "{text}");
        assert!(!busy());
        // An error RESULT keeps its session: the cap was hit, the work stands.
        let bin2 = fake_claude("cap", &fake_conversation(r#"      echo '{"type":"result","subtype":"error_max_turns","is_error":true,"session_id":"s-3"}'"#));
        send("hi".into(), Profile { bin: Some(bin2.clone()), ..Profile::default() }).expect("sent");
        assert_eq!(wait_for(&ended), [Event::Error { text: "claude: error_max_turns".into(), session: Some("s-3".into()) }]);
        // A saved conversation that has gone: said, dropped, asked once more
        // fresh.
        let stale = format!("case \"$*\" in *--resume*) echo 'No conversation found with session ID: gone' >&2; exit 1;; esac\n{}", fake_conversation(ANSWER));
        let bin3 = fake_claude("stale", &stale);
        send("hi".into(), Profile { bin: Some(bin3.clone()), session: Some("gone".into()), ..Profile::default() }).expect("sent");
        let events = wait_for(&ended);
        assert!(matches!(&events[0], Event::Note(n) if n.contains("starting fresh")), "{events:?}");
        assert_eq!(events.last(), Some(&Event::Done { summary: String::new(), session: Some("s-1".into()) }));
        assert_eq!(lines(bin3.parent().unwrap(), "starts").len(), 2);
        stop();
        for b in [bin, bin2, bin3] {
            let _ = std::fs::remove_dir_all(b.parent().unwrap());
        }
        // No claude at all is refused before anything is started.
        let none = Profile { bin: Some("/nonexistent/claude".into()), ..Profile::default() };
        assert!(send("hi".into(), none).unwrap_err().contains("no `claude`"));
        assert!(!busy(), "a refused send leaves nothing running");
    }

    /// Escape ASKS the process to stop, and it stays up: the turn closes as
    /// cancelled, with its conversation, and the next request goes to the
    /// same process.
    #[test]
    fn an_interrupt_stops_the_turn_and_keeps_the_process() {
        assert!(!busy() && !cancel(), "cancelling nothing says so");
        let bin = fake_claude("interrupt", &fake_conversation(ANSWER));
        let dir = bin.parent().unwrap().to_path_buf();
        let profile = Profile { bin: Some(bin), ..Profile::default() };
        send("a long job".into(), profile.clone()).expect("sent");
        wait_for(&|e| e.contains(&Event::Text("working".into())));
        assert!(cancel(), "cancelling a running turn says so");
        assert_eq!(wait_for(&ended), [Event::Cancelled { session: Some("s-1".into()) }], "not an error, and the conversation is named");
        assert!(!busy());
        send("carry on".into(), profile).expect("sent");
        assert!(matches!(wait_for(&ended).last(), Some(Event::Done { .. })));
        assert_eq!(lines(&dir, "starts").len(), 1, "the process the interrupt left running answered");
        assert!(lines(&dir, "stdin")[1].contains("control_request"), "{:?}", lines(&dir, "stdin"));
        stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A call that needs leave is a question, and the answer goes back with
    /// the call as it was asked — or with what to do instead.
    #[test]
    fn a_tool_that_needs_leave_asks_and_waits_for_the_answer() {
        let bin = fake_claude("ask", &fake_conversation(ANSWER));
        let dir = bin.parent().unwrap().to_path_buf();
        let profile = Profile { bin: Some(bin), permission_prompt_tool: Some("stdio".into()), ..Profile::default() };
        let asked = |e: &[Event]| matches!(e.last(), Some(Event::Ask { .. }));
        send("by your leave".into(), profile.clone()).expect("sent");
        let events = wait_for(&asked);
        let [Event::Ask { id, tool, arg, why, .. }] = &events[..] else { panic!("{events:?}") };
        assert_eq!((id.as_str(), tool.as_str(), arg.as_str(), why.as_str()), ("q-1", "Bash", "cargo test", "This command requires approval"));
        assert!(lines(&dir, "argv").join(" ").contains("--permission-prompt-tool stdio"));
        assert!(busy(), "the turn waits");
        assert!(!answer("q-9", true, ""), "not a question that was asked");
        assert!(answer("q-1", true, ""));
        assert!(!answer("q-1", true, ""), "answered once");
        let events = wait_for(&ended);
        assert_eq!(events[0], Event::Result("42".into()));
        let reply: serde_json::Value = serde_json::from_str(lines(&dir, "stdin").last().unwrap()).unwrap();
        assert_eq!(reply["response"]["request_id"], "q-1");
        assert_eq!(reply["response"]["response"]["behavior"], "allow");
        assert_eq!(reply["response"]["response"]["updatedInput"]["command"], "cargo test", "the call goes back as it was asked");

        send("by your leave, again".into(), profile).expect("sent");
        wait_for(&asked);
        assert!(answer("q-1", false, "use cargo check instead"));
        assert_eq!(wait_for(&ended)[0], Event::Result("refused".into()));
        let reply: serde_json::Value = serde_json::from_str(lines(&dir, "stdin").last().unwrap()).unwrap();
        assert_eq!(reply["response"]["response"], serde_json::json!({"behavior": "deny", "message": "use cargo check instead"}));
        stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The host's tools are served on the process's own channel: the
    /// handshake and the list answered here, a call handed to the host and
    /// its answer sent back under the id the call came with.
    #[test]
    fn the_hosts_tools_are_served_on_the_control_channel() {
        let bin = fake_claude("tools", &fake_conversation(ANSWER));
        let dir = bin.parent().unwrap().to_path_buf();
        let tools = r#"[{"name":"buffer_read","description":"Read a buffer.","inputSchema":{"type":"object","properties":{"buffer":{"type":"string"}}}}]"#;
        let profile = Profile {
            bin: Some(bin),
            tools: Some(tools.into()),
            mcp_config: Some(r#"{"mcpServers":{"modeler":{"command":"python3"}}}"#.into()),
            ..Profile::default()
        };
        send("call the tool".into(), profile).expect("sent");
        let events = wait_for(&|e| matches!(e.last(), Some(Event::Call { .. })));
        assert_eq!(events[0], Event::Tool { name: "buffer_read".into(), arg: "main.scm".into() }, "shown without the server's prefix");
        let Some(Event::Call { id, name, args }) = events.last() else { unreachable!() };
        assert_eq!((id.as_str(), name.as_str()), ("c-2", "buffer_read"));
        assert_eq!(args, &[("buffer".to_string(), "main.scm".to_string()), ("from".to_string(), "2".to_string())], "a number arrives as its digits");
        assert!(!events.iter().any(|e| matches!(e, Event::Mcp { .. })), "the protocol is the bridge's business");
        // What the process was told of: the host's server beside the
        // profile's own.
        let argv = lines(&dir, "argv");
        let config: serde_json::Value = serde_json::from_str(&argv[argv.iter().position(|a| a == "--mcp-config").unwrap() + 1]).unwrap();
        assert_eq!(config["mcpServers"]["edit"], serde_json::json!({"type": "sdk", "name": "edit"}));
        assert_eq!(config["mcpServers"]["modeler"]["command"], "python3");
        assert_eq!(lines(&dir, "starts"), ["started tool-search=false"], "its tools are not deferred behind a search");
        // The handshake and the list went back before any call did.
        // (The stand-in keeps each line as it reads it, a moment after.)
        let t0 = std::time::Instant::now();
        while lines(&dir, "stdin").len() < 3 {
            assert!(t0.elapsed() < std::time::Duration::from_secs(10), "{:?}", lines(&dir, "stdin"));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let sent: Vec<serde_json::Value> = lines(&dir, "stdin").iter().skip(1).map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(sent[0]["response"]["request_id"], "c-0");
        assert_eq!(sent[0]["response"]["response"]["mcp_response"]["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(sent[1]["response"]["response"]["mcp_response"]["result"]["tools"][0]["name"], "buffer_read");
        assert_eq!(sent[1]["response"]["response"]["mcp_response"]["id"], 2);

        assert!(!tool_result("c-9", "x", false), "not a call that was made");
        assert!(tool_result("c-2", "(define b 2)", false));
        assert!(!tool_result("c-2", "again", false), "answered once");
        assert!(matches!(wait_for(&ended).last(), Some(Event::Done { .. })));
        let answer: serde_json::Value = serde_json::from_str(&lines(&dir, "tool-answer")[0]).unwrap();
        let answer = &answer["response"]["response"]["mcp_response"];
        assert_eq!(answer["id"], 3, "under the id the call came with");
        assert_eq!(answer["result"], serde_json::json!({"content": [{"type": "text", "text": "(define b 2)"}], "isError": false}));
        stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A process that does not answer the interrupt is killed when the
    /// grace runs out; the turn still closes as cancelled, and the next
    /// request starts another.
    #[test]
    fn a_process_deaf_to_the_interrupt_is_killed() {
        agent().grace_ms.store(150, Ordering::SeqCst);
        let deaf = "read -r line\necho '{\"type\":\"assistant\",\"message\":{\"id\":\"m\",\"content\":[{\"type\":\"text\",\"text\":\"working\"}]}}'\nexec sleep 30";
        let bin = fake_claude("deaf", deaf);
        let dir = bin.parent().unwrap().to_path_buf();
        let profile = Profile { bin: Some(bin), ..Profile::default() };
        send("a job".into(), profile.clone()).expect("sent");
        wait_for(&|e| e.contains(&Event::Text("working".into())));
        let asked = std::time::Instant::now();
        assert!(cancel());
        assert_eq!(wait_for(&ended), [Event::Cancelled { session: None }]);
        assert!(asked.elapsed() < std::time::Duration::from_secs(5), "it did not wait out the sleep");
        assert!(!busy());
        send("again".into(), profile).expect("sent");
        wait_for(&|e| e.contains(&Event::Text("working".into())));
        assert_eq!(lines(&dir, "starts").len(), 2, "a new process");
        // A new conversation ends the process outright, turn and all.
        stop();
        assert_eq!(wait_for(&ended), [Event::Cancelled { session: None }]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The bridge against the REAL `claude`, on the login this machine
    /// holds — spends three small turns, so it is run by hand:
    /// `cargo test -p edit --lib the_real_claude -- --ignored --nocapture`.
    #[test]
    #[ignore = "spends real turns on the logged-in Claude subscription"]
    fn the_real_claude_holds_a_conversation_and_asks_leave() {
        let dir = std::env::temp_dir().join(format!("edit-agent-real-{}", std::process::id()));
        let profile = Profile {
            cwd: Some(dir.clone()),
            model: Some("haiku".into()),
            permission_mode: Some("acceptEdits".into()),
            permission_prompt_tool: Some("stdio".into()),
            ..Profile::default()
        };
        let began = std::time::Instant::now();
        send("Remember the number 4471. Reply with the single word: ok".into(), profile.clone()).expect("sent");
        let first = wait_for(&ended);
        let Some(Event::Done { session: Some(session), .. }) = first.last() else { panic!("{first:#?}") };
        eprintln!("turn 1 (starts the process): {:.2}s", began.elapsed().as_secs_f64());
        assert!(first.iter().any(|e| matches!(e, Event::Delta(_))), "the reply arrived as it was written: {first:#?}");

        let began = std::time::Instant::now();
        send("Which number? Just the number.".into(), Profile { session: Some(session.clone()), ..profile.clone() }).expect("sent");
        let second = wait_for(&ended);
        eprintln!("turn 2 (same process): {:.2}s", began.elapsed().as_secs_f64());
        assert!(second.iter().any(|e| matches!(e, Event::Text(t) if t.contains("4471"))), "{second:#?}");
        assert_eq!(second.last(), Some(&Event::Done { summary: String::new(), session: Some(session.clone()) }), "one conversation");

        send("Run the shell command `python3 -c \"print(6*7)\"` with Bash and tell me what it printed.".into(), profile).expect("sent");
        let asked = wait_for(&|e| matches!(e.last(), Some(Event::Ask { .. })));
        let Some(Event::Ask { id, tool, arg, .. }) = asked.last() else { unreachable!() };
        assert_eq!(tool, "Bash");
        assert!(arg.contains("python3"), "{arg}");
        assert!(answer(id, true, ""));
        let third = wait_for(&ended);
        assert!(third.iter().any(|e| matches!(e, Event::Result(r) if r.contains("42"))), "{third:#?}");
        stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn auth_is_the_subscription_login_and_forwards_the_pasted_code() {
        // login: a URL, a prompt with NO newline, then it waits for a line.
        let bin = fake_claude(
            "auth",
            "case \"$2\" in\n login) echo 'Opening browser: https://example.test/x'; printf 'Paste code here if prompted > '; read code; echo \"got $code\";;\n status) cat \"$D/status\";;\n logout) exit 3;;\nesac",
        );
        let dir = bin.parent().unwrap().to_path_buf();
        let profile = Profile { bin: Some(bin), ..Profile::default() };
        auth("login", &profile).expect("started");
        let events = wait_for(&|e| e.iter().any(|e| matches!(e, Event::Auth(l) if l.starts_with("Paste code"))));
        assert_eq!(events[0], Event::Auth("Opening browser: https://example.test/x".into()));
        assert_eq!(lines(&dir, "argv"), ["auth", "login", "--claudeai"], "the subscription, said outright");
        assert!(auth_pending());
        assert!(auth_input("  ABC-123 \n"));
        let events = wait_for(&|e| e.iter().any(|e| matches!(e, Event::AuthDone(_))));
        assert_eq!(events, vec![Event::Auth("got ABC-123".into()), Event::AuthDone("logged in — ask away".into())]);
        assert!(!auth_pending() && !auth_input("late"));

        let status = |json: &str| {
            std::fs::write(dir.join("status"), json).unwrap();
            auth("status", &profile).unwrap();
            wait_for(&|e| !e.is_empty())
        };
        assert_eq!(
            status(r#"{"loggedIn":true,"authMethod":"claude.ai","email":"k@example.test","subscriptionType":"max"}"#),
            [Event::AuthDone("logged in as k@example.test (max)".into())],
            "said in words; the JSON is not relayed"
        );
        let [Event::AuthDone(console)] = &status(r#"{"loggedIn":true,"authMethod":"console","email":"k@example.test"}"#)[..] else { panic!() };
        assert!(console.contains("NOT your Claude subscription"), "an API login is not passed off as the subscription: {console}");
        assert_eq!(status(r#"{"loggedIn":false}"#), [Event::AuthDone("not logged in — /login to sign in".into())]);
        auth("logout", &profile).unwrap();
        let events = wait_for(&|e| !e.is_empty());
        assert!(matches!(&events[0], Event::AuthError(t) if t.contains("claude auth logout exited")), "{events:?}");
        assert!(auth("whoami", &profile).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
