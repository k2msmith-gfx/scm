//! The agent pane's prims (docs/agent-panel-plan.md, A2): one call each
//! into `crate::agent`, the bridge to Claude Code. A PROFILE crosses as a
//! flat list — `("cwd" "/proj" "model" "opus" …)` — so that no shape of
//! pair or list has to be agreed on twice; 25-agent.scm flattens its alist.

use scm::Val;

use super::{val_text, val_textv};
use crate::agent::{self, Event, Profile};

fn text(s: impl Into<String>) -> Val {
    Val::Str(s.into())
}

/// A session id, or #f: the conversation has none yet.
fn session(s: Option<String>) -> Val {
    s.map(Val::Str).unwrap_or(Val::Bool(false))
}

/// An event as the row Scheme reads: its kind, then what it carries.
fn row(e: Event) -> Val {
    Val::List(match e {
        Event::Delta(t) => vec![text("delta"), text(t)],
        Event::Text(t) => vec![text("text"), text(t)],
        Event::Tool { name, arg } => vec![text("tool"), text(name), text(arg)],
        Event::Result(t) => vec![text("result"), text(t)],
        Event::Denied { tool, why } => vec![text("denied"), text(tool), text(why)],
        // (The call's input stays in the kernel: %agent-answer hands it back.)
        Event::Ask { id, tool, arg, why, .. } => vec![text("ask"), text(id), text(tool), text(arg), text(why)],
        Event::Call { id, name, args } => {
            let mut row = vec![text("call"), text(id), text(name)];
            row.extend(args.into_iter().flat_map(|(k, v)| [text(k), text(v)]));
            row
        }
        // Answered by the bridge; never queued.
        Event::Mcp { .. } => vec![text("note"), text("")],
        Event::Metrics(t) => vec![text("metrics"), text(t)],
        Event::Note(t) => vec![text("note"), text(t)],
        Event::Done { summary, session: s } => vec![text("done"), text(summary), session(s)],
        Event::Error { text: t, session: s } => vec![text("error"), text(t), session(s)],
        Event::Cancelled { session: s } => vec![text("cancelled"), text(""), session(s)],
        Event::Auth(t) => vec![text("auth"), text(t)],
        Event::AuthDone(t) => vec![text("auth-done"), text(t)],
        Event::AuthError(t) => vec![text("auth-error"), text(t)],
    })
}

/// Byte offset AT of TEXT as the editor addresses it: (line, byte in line).
fn place(text: &str, at: usize) -> (usize, usize) {
    let before = &text[..at];
    (before.matches('\n').count(), at - before.rfind('\n').map_or(0, |i| i + 1))
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    // `(%agent-send prompt profile)` — start a turn: PROMPT to the agent
    // PROFILE describes. Returns #t at once; the turn arrives through
    // %agent-poll. Raises with the reason when it cannot start (no claude,
    // a turn already running, a profile key it does not know).
    interp.define_fn("%agent-send", |args: &[Val]| {
        let prompt = val_text(args, 0).ok_or("expected a prompt")?;
        agent::send(prompt, Profile::from_pairs(&val_textv(args, 1))?)?;
        Ok(Val::Bool(true))
    })?;
    // `(%agent-poll)` — every event since the last call, oldest first, each
    // a list: ("delta" T) ("text" T) ("tool" NAME ARG) ("result" T) ("denied" TOOL WHY)
    // ("ask" ID TOOL ARG WHY) ("call" ID NAME KEY VALUE …)
    // ("metrics" T) ("note" T) ("done" SUMMARY SESSION) ("error" T SESSION)
    // ("cancelled" "" SESSION) ("auth" LINE) ("auth-done" T) ("auth-error" T). SESSION
    // is the id that continues the conversation, or #f.
    interp.define_fn("%agent-poll", |_args: &[Val]| Ok(Val::List(agent::poll().into_iter().map(row).collect())))?;
    // `(%agent-busy?)` — is a turn running?
    interp.define_fn("%agent-busy?", |_args: &[Val]| Ok(Val::Bool(agent::busy())))?;
    // `(%agent-cancel)` — ask the turn being answered to stop; #f when
    // there was none. The conversation's process stays up.
    interp.define_fn("%agent-cancel", |_args: &[Val]| Ok(Val::Bool(agent::cancel())))?;
    // `(%agent-answer id allow? message)` — answer the question an "ask"
    // event put: allow the call, or refuse it with MESSAGE, which the agent
    // is told. #f when that question is no longer open.
    interp.define_fn("%agent-answer", |args: &[Val]| {
        let id = val_text(args, 0).unwrap_or_default();
        let allow = !matches!(args.get(1), Some(Val::Bool(false)) | None);
        Ok(Val::Bool(agent::answer(&id, allow, &val_text(args, 2).unwrap_or_default())))
    })?;
    // `(%agent-tool-result id text failed?)` — answer the "call" event ID:
    // TEXT is what the tool says. #f when that call is no longer open.
    interp.define_fn("%agent-tool-result", |args: &[Val]| {
        let id = val_text(args, 0).unwrap_or_default();
        let failed = !matches!(args.get(2), Some(Val::Bool(false)) | None);
        Ok(Val::Bool(agent::tool_result(&id, &val_text(args, 1).unwrap_or_default(), failed)))
    })?;
    // `(%json-string text)` — TEXT as a JSON string, quotes and all: what a
    // tool's description is written with.
    interp.define_fn("%json-string", |args: &[Val]| Ok(Val::Str(serde_json::Value::String(val_text(args, 0).unwrap_or_default()).to_string())))?;
    // `(%editor-edit id old new)` — in pane ID's buffer, replace the ONE
    // place OLD occurs with NEW, as one undo step: the number of places OLD
    // occurs, and the edit made only when that is 1.
    interp.define_fn("%editor-edit", |args: &[Val]| {
        let id = super::val_num(args, 0) as u32;
        let (old, new) = (val_text(args, 1).unwrap_or_default(), val_text(args, 2).unwrap_or_default());
        let text = crate::editorpane::text(id);
        let found = if old.is_empty() { 0 } else { text.matches(&old).count() };
        if found == 1
            && let Some(at) = text.find(&old)
        {
            let (from, to) = (place(&text, at), place(&text, at + old.len()));
            crate::editorpane::replace_range(id, from.0, from.1, to.0, to.1, &new);
        }
        Ok(Val::Int(found as i64))
    })?;
    // `(%editor-rewrite id text)` — replace the whole of pane ID's buffer
    // with TEXT as one undo step (where %editor-set-text is a LOAD, and
    // forgets the undo).
    interp.define_fn("%editor-rewrite", |args: &[Val]| {
        let id = super::val_num(args, 0) as u32;
        let text = crate::editorpane::text(id);
        let end = place(&text, text.len());
        crate::editorpane::replace_range(id, 0, 0, end.0, end.1, &val_text(args, 1).unwrap_or_default());
        Ok(Val::Bool(true))
    })?;
    // `(%agent-usage)` — what the conversation has used and what the plan
    // has left, as lines: /usage.
    interp.define_fn("%agent-usage", |_args: &[Val]| Ok(Val::List(agent::usage().into_iter().map(Val::Str).collect())))?;
    // `(%agent-stop)` — end the conversation's process: what a NEW
    // conversation needs, so that its first request is not the old one's
    // next.
    interp.define_fn("%agent-stop", |_args: &[Val]| {
        agent::stop();
        Ok(Val::Bool(false))
    })?;
    // `(%agent-available? profile)` — is there a `claude` for PROFILE to run?
    interp.define_fn("%agent-available?", |args: &[Val]| {
        Ok(Val::Bool(agent::claude_bin(&Profile::from_pairs(&val_textv(args, 0))?).is_some()))
    })?;
    // `(%agent-auth action profile)` — run `claude auth ACTION` ("login",
    // "logout", "status"); what it says arrives through %agent-poll.
    interp.define_fn("%agent-auth", |args: &[Val]| {
        let action = val_text(args, 0).ok_or("expected login, logout or status")?;
        agent::auth(&action, &Profile::from_pairs(&val_textv(args, 1))?)?;
        Ok(Val::Bool(true))
    })?;
    // `(%agent-auth-pending?)` — is a login waiting for its pasted code?
    interp.define_fn("%agent-auth-pending?", |_args: &[Val]| Ok(Val::Bool(agent::auth_pending())))?;
    // `(%agent-auth-input text)` — hand TEXT (the code) to the pending login.
    interp.define_fn("%agent-auth-input", |args: &[Val]| Ok(Val::Bool(agent::auth_input(&val_text(args, 0).unwrap_or_default()))))?;
    Ok(vec![
        "%agent-send",
        "%agent-poll",
        "%agent-busy?",
        "%agent-cancel",
        "%agent-stop",
        "%agent-usage",
        "%agent-answer",
        "%agent-tool-result",
        "%json-string",
        "%editor-edit",
        "%editor-rewrite",
        "%agent-available?",
        "%agent-auth",
        "%agent-auth-pending?",
        "%agent-auth-input",
    ])
}

#[cfg(test)]
mod tests {
    use scm::Val;

    fn eval(interp: &mut scm::Interp, src: &str) -> Val {
        let r = interp.eval(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        interp.value(&r).expect("value")
    }

    fn texts(v: Val) -> Vec<String> {
        let Val::List(items) = v else { return Vec::new() };
        items.into_iter().map(|v| if let Val::Str(s) = v { s } else { format!("{v:?}") }).collect()
    }

    /// Call (remote-poll) — what a host's wake does — until DONE says the
    /// transcript has what was waited for.
    fn settle(interp: &mut scm::Interp, done: &str) -> Vec<String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            eval(interp, "(remote-poll)");
            if eval(interp, done) != Val::Bool(false) {
                return texts(eval(interp, "(rows)"));
            }
            assert!(std::time::Instant::now() < deadline, "never came: {done}\n{:#?}", texts(eval(interp, "(rows)")));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// One answered request, as the stand-in prints it.
    const TURN: &str = r#"cat <<'EOF'
{"type":"system","subtype":"init","session_id":"s-1"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"I'll fix"}}}
{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"I'll fix it.\nOne moment."}]}}
{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/p/main.rs"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"updated"}]}}
{"type":"system","subtype":"permission_denied","tool_name":"Bash","message":"This command requires approval"}
{"type":"assistant","message":{"id":"m3","content":[{"type":"text","text":"Fixed."}]}}
{"type":"result","is_error":false,"result":"Fixed.","session_id":"s-1","num_turns":2}
EOF"#;

    /// The pane on a bare interpreter, against a stand-in `claude`: a
    /// request typed at agent> becomes a turn, the turn's steps become rows,
    /// the conversation is resumed on the next request and forgotten on C-l,
    /// /status reads the account, and Escape's interrupt says so quietly.
    #[test]
    fn the_agent_pane_holds_a_conversation() {
        let mut interp = scm::Interp::new().expect("interp");
        super::super::install(&mut interp, &mut |_| {}).expect("install");
        let (src, host) = super::super::shell_source("host/prelude").expect("host prelude");
        interp.eval_named(&src, &host).expect("host prelude loads");
        eval(&mut interp, r#"
            (define *typing* #f)
            (define *focused* 0)
            (define (%focused-pane) *focused*)
            (define (%focus-pane id) (set! *focused* id))
            (define (%typing-pane) *typing*)
            (define (%pane-at x y) (list 0 "text"))
            (define (%os-clock) 0)
            (define (%hold-typing id) (set! *typing* id))
            (define (%release-typing) (set! *typing* #f))
            (define (%os-realpath p) p)"#);
        for name in super::super::SHELL {
            let (src, path) = super::super::shell_source(name).expect("read");
            interp.eval_named(&src, &path).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let auth = r#"if [ "$1" = auth ]; then echo '{"loggedIn":true,"authMethod":"claude.ai","email":"k@example.test","subscriptionType":"max"}'; exit 0; fi"#;
        let bin = crate::agent::fake_claude("pane", &format!("{auth}\n{}", crate::agent::fake_conversation(TURN)));
        let dir = bin.parent().unwrap().to_string_lossy().into_owned();
        let file = |name: &str| std::fs::read_to_string(format!("{dir}/{name}")).unwrap_or_default();
        let argv = || file("argv");
        let starts = || file("starts").lines().count();
        eval(&mut interp, &format!(r#"
            (set! *pane-id* (lambda (key) (case key ((:minibuffer) 7) ((:editor) 3) ((:agent) 9) (else #f))))
            (bind-editor-keys :minibuffer)
            (bind-agent-keys)
            (register-buffer (%editor-active-buffer 3) #f)
            (define shown #f)
            (set! *agent-shown* (lambda (id) (set! shown id)))
            (define ended '())
            (set! *agent-turn-ended* (lambda (status) (set! ended (cons status ended))))
            (define bin "{dir}/claude")
            (set! *agent-profile*
              (lambda () (agent-profile-extend (agent-code-profile) :bin bin :cwd "{dir}/proj" :model "haiku")))
            (define (rows) (map (lambda (i) (%pane-row-text 9 i)) (iota (%pane-row-count 9))))"#));

        // A profile is built on the code profile: a setting replaces, #f
        // removes, :arg adds; and it flattens to what the kernel reads.
        assert_eq!(
            texts(eval(&mut interp, r#"(%agent-flat (agent-profile-extend '((:model . "opus") (:denied-tools . "Bash"))
                                                      :model "sonnet" :denied-tools #f :strict-mcp #t :arg "--a" :arg "--b"))"#)),
            ["model", "sonnet", "strict-mcp", "1", "arg", "--a", "arg", "--b"]
        );
        assert_eq!(eval(&mut interp, "(%alist-ref (agent-code-profile) :permission-prompt-tool #f)"), Val::Str("stdio".into()), "it asks, here");
        assert_eq!(eval(&mut interp, "(%alist-ref (agent-code-profile) :denied-tools #f)"), Val::Bool(false), "the shell is not denied: it is asked for");

        // Typed at agent>, sent on return.
        eval(&mut interp, r#"(focus-agent) (on-text "f") (on-text "i") (on-text "x") (on-key "return")"#);
        assert_eq!(eval(&mut interp, "shown"), Val::Int(9), "the host was asked to show the pane");
        assert_eq!(eval(&mut interp, "*typing*"), Val::Int(9));
        assert_eq!(texts(eval(&mut interp, "(rows)")), ["you> fix", "thinking…"], "it says it is working");
        let rows = settle(&mut interp, "(pair? ended)");
        assert_eq!(rows[0], "you> fix");
        // (No row for the Edit's "updated": a buffer reloading says it.)
        assert_eq!(&rows[1..5], ["I'll fix it.", "One moment.", "→ Edit /p/main.rs", "✗ Bash was not allowed — This command requires approval"]);
        assert_eq!(rows[5], "Fixed.");
        assert!(rows[6].starts_with("· ") && rows[6].contains("2 turns · 1 tools"), "the meter: {rows:?}");
        assert_eq!(rows.len(), 7, "the summary only repeated the last text, so it is not drawn again: {rows:?}");
        assert_eq!(eval(&mut interp, "ended"), Val::List(vec![Val::Str("done".into())]));
        let first = argv();
        assert!(first.contains("--permission-mode\nacceptEdits\n") && first.contains("--permission-prompt-tool\nstdio\n"), "{first}");
        assert!(first.contains("--model\nhaiku\n") && !first.contains("--resume"), "{first}");
        assert_eq!(starts(), 1);
        // What is sent is the request, with what the editor shows in front.
        let sent: serde_json::Value = serde_json::from_str(file("stdin").lines().next().unwrap()).unwrap();
        let sent = sent["message"]["content"].as_str().unwrap().to_string();
        assert!(sent.starts_with("<editor>\nfile: ") && sent.ends_with("</editor>\n\nfix"), "{sent}");

        // The next request continues that conversation, in the same
        // process; what the host prepares is what is sent.
        eval(&mut interp, r#"(set! *agent-prepare* (lambda (text) (string-append "Scene: empty\nRequest: " text)))
                             (set! ended '())
                             (agent-submit "and the other one")"#);
        settle(&mut interp, "(pair? ended)");
        assert_eq!(starts(), 1, "no second process");
        assert!(file("stdin").contains(r"Scene: empty\nRequest: and the other one"), "{}", file("stdin"));

        // /usage: what the conversation has used so far.
        eval(&mut interp, r#"(agent-submit "/usage")"#);
        let rows = texts(eval(&mut interp, "(rows)"));
        assert!(rows.iter().any(|r| r.starts_with("this conversation: 4 turns · 2 tools")), "{rows:?}");

        // The account, from the pane; a command it does not know.
        eval(&mut interp, r#"(agent-submit "/status")"#);
        let rows = settle(&mut interp, r#"(member "logged in as k@example.test (max)" (rows))"#);
        assert_eq!(rows[rows.len() - 2], "you> /status");
        eval(&mut interp, r#"(agent-submit "/nope")"#);
        assert!(texts(eval(&mut interp, "(rows)")).last().unwrap().starts_with("unknown command /nope"));

        // /model: another process, resumed into the SAME conversation.
        eval(&mut interp, r#"(agent-submit "/model sonnet") (set! ended '()) (agent-submit "quicker")"#);
        settle(&mut interp, "(pair? ended)");
        assert_eq!(starts(), 2);
        assert!(argv().contains("--model\nsonnet\n") && argv().contains("--resume\ns-1\n"), "{}", argv());

        // C-l: an empty transcript and a conversation that is not resumed.
        eval(&mut interp, r#"(set! *focused* 9) (on-key "C-l")"#);
        assert_eq!(texts(eval(&mut interp, "(rows)")), ["— new conversation —"]);
        eval(&mut interp, r#"(set! ended '()) (agent-submit "afresh")"#);
        settle(&mut interp, "(pair? ended)");
        assert_eq!(starts(), 3, "a new conversation is a new process");
        assert!(!argv().contains("--resume"), "{}", argv());

        // Escape's interrupt: the turn stops, the pane says so quietly, and
        // the process that was interrupted answers the next request.
        eval(&mut interp, r#"(set! ended '()) (agent-submit "a long job")"#);
        settle(&mut interp, r#"(member "working" (rows))"#);
        eval(&mut interp, r#"(agent-submit "hurry")"#);
        assert_eq!(texts(eval(&mut interp, "(rows)")).last().unwrap(), "still answering — Escape interrupts it");
        assert_eq!(eval(&mut interp, "(agent-typing?)"), Val::Bool(true));
        assert_eq!(eval(&mut interp, "(agent-interrupt)"), Val::Bool(true));
        let rows = settle(&mut interp, "(pair? ended)");
        assert_eq!(rows.last().unwrap(), "— interrupted —");
        assert_eq!(eval(&mut interp, "ended"), Val::List(vec![Val::Str("cancelled".into())]));
        assert_eq!(eval(&mut interp, "(agent-interrupt)"), Val::Bool(false), "nothing left to stop");
        eval(&mut interp, r#"(set! ended '()) (agent-submit "carry on")"#);
        settle(&mut interp, "(pair? ended)");
        assert_eq!(starts(), 3, "the same process");

        // Leave: a call the agent may not make unasked is a question in
        // the pane, and the next line typed is its answer.
        let last_sent = || serde_json::from_str::<serde_json::Value>(file("stdin").lines().last().unwrap()).unwrap();
        eval(&mut interp, r#"(set! ended '()) (agent-submit "by your leave")"#);
        let rows = settle(&mut interp, "(pair? *agent-asks*)");
        assert_eq!(rows[rows.len() - 2], "? Bash cargo test", "{rows:?}");
        assert!(rows[rows.len() - 1].contains("y allow · n deny · a allow every Bash call"), "{rows:?}");
        eval(&mut interp, r#"(agent-submit "once")"#);
        let rows = settle(&mut interp, "(pair? ended)");
        assert!(rows.contains(&"→ allowed, this once".to_string()) && rows.contains(&"42".to_string()), "{rows:?}");
        // Anything that is not yes or no refuses, and is what it is told.
        eval(&mut interp, r#"(set! ended '()) (agent-submit "leave, again")"#);
        settle(&mut interp, "(pair? *agent-asks*)");
        eval(&mut interp, r#"(agent-submit "use cargo check instead")"#);
        let rows = settle(&mut interp, "(pair? ended)");
        assert!(rows.contains(&"→ refused".to_string()) && rows.contains(&"refused".to_string()), "{rows:?}");
        assert_eq!(last_sent()["response"]["response"]["message"], "use cargo check instead");
        // "a": this call, and every later one of that tool, unasked.
        eval(&mut interp, r#"(set! ended '()) (agent-submit "leave, a third time")"#);
        settle(&mut interp, "(pair? *agent-asks*)");
        eval(&mut interp, r#"(agent-submit "a")"#);
        settle(&mut interp, "(pair? ended)");
        eval(&mut interp, r#"(set! ended '()) (agent-submit "leave, once more")"#);
        let rows = settle(&mut interp, "(pair? ended)");
        assert!(rows.contains(&"✓ Bash cargo test".to_string()), "allowed without asking: {rows:?}");
        assert_eq!(last_sent()["response"]["response"]["behavior"], "allow");

        // A reply is drawn as it is written: pieces grow the last row, a
        // newline starts another, and the whole message lands on the same
        // rows — onto the "thinking…" row when one is waiting.
        eval(&mut interp, r#"(%pane-clear 9)
                             (agent-append "thinking…" :dim) (set! *agent-pending* 0)
                             (agent-event '("delta" "Two li"))"#);
        assert_eq!(texts(eval(&mut interp, "(rows)")), ["Two li"]);
        eval(&mut interp, r#"(agent-event '("delta" "nes,\nthe sec"))"#);
        assert_eq!(texts(eval(&mut interp, "(rows)")), ["Two lines,", "the sec"]);
        eval(&mut interp, r#"(agent-event '("text" "Two lines,\nthe second.")) (agent-event '("tool" "Read" "/p/a.rs"))
                             (agent-event '("delta" "Then")) (agent-event '("text" "Then more."))"#);
        assert_eq!(texts(eval(&mut interp, "(rows)")), ["Two lines,", "the second.", "→ Read /p/a.rs", "Then more."]);

        // No claude: the pending row becomes the reason, in the pane.
        eval(&mut interp, r#"(set! bin "/nonexistent/claude") (agent-submit "anyone?")"#);
        let rows = texts(eval(&mut interp, "(rows)"));
        assert!(rows.last().unwrap().starts_with("[agent] no `claude` on PATH"), "{rows:?}");
        assert!(!rows.contains(&"thinking…".to_string()), "{rows:?}");
        crate::agent::stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
