//! The REPL as a protocol — plan §8, stage S8.
//!
//! > the terminal prompt is **one client among several** — the others are
//! > Emacs over TCP, the modeler's editor pane, SLIME sends, and the agent
//! > pane … Building the loop first and bolting a socket on later is how the
//! > Janet side ended up with an eval channel per consumer.
//!
//! So [`Session`] is the whole REPL and [`Session::handle`] is its only door.
//! `scm-cli`'s prompt and its socket server are both clients of it, and the
//! exit criterion is that they cannot diverge, because there is nothing for
//! them to diverge *in*: a client's entire job is to turn bytes into a
//! [`Request`] and a [`Response`] back into bytes.
//!
//! ## The wire format is s-expressions, and the framing is the reader
//!
//! A request is a keyword-keyed form — `(:op eval :id 1 :code "(+ 1 2)")` —
//! for two reasons. The server already has a reader and a writer for exactly
//! this, so no second syntax needs maintaining; and **message framing comes
//! free from [`crate::parse_status`]**, the same incomplete-vs-invalid
//! distinction S1 built for multi-line entry (§8 obligation 5). A client
//! reads until the text it has is a complete datum. No length prefixes, no
//! newline escaping, and a multi-line form needs no special case.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::embed::{Description, Error, Interp};

/// What a client asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Evaluate source against the persistent interactive environment
    /// (§8 obligation 1). `source` names the input in a backtrace -- a file
    /// path, or whatever a client calls the buffer the form came from.
    Eval { code: String, source: Option<String> },
    /// Global names beginning with a prefix.
    Complete { prefix: String },
    /// What a name is: signature, docstring, kind, current value.
    Describe { name: String },
    /// Is this text a complete datum, more to come, or wrong? What an editor
    /// asks before deciding whether `RET` submits.
    Parse { code: String },
    /// Stop whatever is evaluating. Handled **out of band** -- see
    /// [`Session::interrupt_handle`].
    Interrupt,
    /// Liveness, and the version handshake.
    Ping,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Echoed in the response, so a client may have several in flight.
    pub id: u64,
    pub op: Op,
}

/// How a form's text parsed, for [`Op::Parse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    Complete,
    Incomplete,
    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A value, printed, plus whatever the form printed on the way.
    Value { value: String, output: String },
    /// A refusal. `output` is still carried: what a form printed before it
    /// broke is usually the most useful part.
    Failed {
        message: String,
        output: String,
        /// Whether the *host* stopped it (interrupt, fuel, heap cap) rather
        /// than the script failing. A client should not offer "retry" for
        /// these.
        host_stopped: bool,
        /// The pending calls, innermost first, each `"(form)  at src:line:col"`.
        /// Empty when nothing was pending -- a read error, or a top-level
        /// atom.
        backtrace: Vec<String>,
    },
    Candidates(Vec<String>),
    /// `None` when the name is unbound.
    Described(Option<Description>),
    Framed(Framing),
    Pong { version: String },
    /// The request itself could not be understood.
    Malformed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub id: u64,
    pub outcome: Outcome,
}

/// A persistent interpreter plus the REPL's operations over it.
pub struct Session {
    interp: Interp,
}

impl Session {
    pub fn new() -> Result<Self, Error> {
        Ok(Session {
            interp: Interp::new()?,
        })
    }

    /// The flag that stops a running evaluation.
    ///
    /// **This is why `:interrupt` is out of band.** A server that queued an
    /// interrupt behind the evaluation it was meant to stop would never
    /// deliver it: the one interpreter is single-threaded, so the eval holds
    /// it until it finishes. A client's reading thread sets this flag
    /// directly instead, and the eval notices at its next safepoint.
    pub fn interrupt_handle(&self) -> Arc<AtomicBool> {
        self.interp.interrupt_handle()
    }

    pub fn interp_mut(&mut self) -> &mut Interp {
        &mut self.interp
    }

    /// The one door. Every client goes through here.
    pub fn handle(&mut self, req: Request) -> Response {
        let outcome = match req.op {
            Op::Ping => Outcome::Pong {
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            Op::Parse { code } => Outcome::Framed(match crate::parse_status(&code) {
                crate::ParseStatus::Complete => Framing::Complete,
                crate::ParseStatus::Incomplete => Framing::Incomplete,
                crate::ParseStatus::Invalid(_) => Framing::Invalid,
            }),
            Op::Complete { prefix } => Outcome::Candidates(self.interp.complete(&prefix)),
            Op::Describe { name } => Outcome::Described(self.interp.describe(&name)),
            // Reaching here means the interrupt was queued after all, which a
            // correct server avoids. Setting the flag is still right: the next
            // evaluation stops immediately, which is visible and recoverable,
            // where silently ignoring it would not be.
            Op::Interrupt => {
                self.interrupt_handle()
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                Outcome::Pong {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                }
            }
            Op::Eval { code, source } => {
                let (result, output) = self
                    .interp
                    .eval_capturing_named(&code, source.as_deref());
                match result {
                    Ok(v) => {
                        let value = self
                            .interp
                            .pretty(&v, 80)
                            .unwrap_or_else(|e| format!("#<unprintable: {e}>"));
                        Outcome::Value { value, output }
                    }
                    Err(e) => Outcome::Failed {
                        message: format!("{e}"),
                        output,
                        host_stopped: matches!(
                            e,
                            Error::Interrupted | Error::OutOfFuel | Error::OutOfMemory
                        ),
                        backtrace: self.interp.backtrace(),
                    },
                }
            }
        };
        Response {
            id: req.id,
            outcome,
        }
    }
}

// ---- the wire format ----------------------------------------------------

impl Request {
    /// Parse `(:op eval :id 1 :code "…")`.
    ///
    /// Read with scm's own reader, on a scratch engine: a malformed request
    /// must not intern anything into the live session, and a client is not a
    /// trusted source.
    pub fn parse(text: &str) -> Result<Request, String> {
        let mut e = crate::Engine::new();
        let v = crate::read_one(text, &mut e)
            .map_err(|err| format!("{err}"))?
            .ok_or("an empty request")?;
        let items = proper_list(&e, v).ok_or("a request must be a proper list")?;
        let mut id = 0u64;
        let mut op: Option<String> = None;
        let mut code: Option<String> = None;
        let mut prefix: Option<String> = None;
        let mut name: Option<String> = None;
        let mut i = 0;
        while i < items.len() {
            let crate::Value::Keyword(k) = items[i] else {
                return Err("a request is :key value pairs".into());
            };
            let key = e.interner.kw_name(k).to_string();
            let Some(&val) = items.get(i + 1) else {
                return Err(format!(":{key} has no value"));
            };
            match key.as_str() {
                "id" => {
                    let crate::Value::Fixnum(n) = val else {
                        return Err(":id must be an integer".into());
                    };
                    id = n.max(0) as u64;
                }
                "op" => op = Some(text_of(&e, val).ok_or(":op must be a symbol or string")?),
                "code" => code = Some(text_of(&e, val).ok_or(":code must be a string")?),
                "prefix" => prefix = Some(text_of(&e, val).unwrap_or_default()),
                "name" => name = Some(text_of(&e, val).ok_or(":name must be a symbol or string")?),
                // Unknown keys are ignored rather than refused, so a newer
                // client talking to an older server still works.
                _ => {}
            }
            i += 2;
        }
        let op = match op.as_deref() {
            Some("eval") => Op::Eval {
                code: code.ok_or("eval needs :code")?,
                source: name,
            },
            Some("complete") => Op::Complete {
                prefix: prefix.unwrap_or_default(),
            },
            Some("describe") => Op::Describe {
                name: name.ok_or("describe needs :name")?,
            },
            Some("parse") => Op::Parse {
                code: code.ok_or("parse needs :code")?,
            },
            Some("interrupt") => Op::Interrupt,
            Some("ping") => Op::Ping,
            Some(other) => return Err(format!("unknown op `{other}`")),
            None => return Err("a request needs :op".into()),
        };
        Ok(Request { id, op })
    }
}

impl Response {
    /// The response as one datum, which is also its own frame.
    pub fn to_wire(&self) -> String {
        let mut out = format!("(:id {}", self.id);
        match &self.outcome {
            Outcome::Value { value, output } => {
                out.push_str(" :status ok :value ");
                push_string(&mut out, value);
                out.push_str(" :output ");
                push_string(&mut out, output);
            }
            Outcome::Failed {
                message,
                output,
                host_stopped,
                backtrace,
            } => {
                out.push_str(" :status error :message ");
                push_string(&mut out, message);
                out.push_str(" :output ");
                push_string(&mut out, output);
                out.push_str(if *host_stopped {
                    " :host-stopped #t"
                } else {
                    " :host-stopped #f"
                });
                out.push_str(" :backtrace (");
                for (i, f) in backtrace.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    push_string(&mut out, f);
                }
                out.push(')');
            }
            Outcome::Candidates(names) => {
                out.push_str(" :status ok :candidates (");
                for (i, n) in names.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    push_string(&mut out, n);
                }
                out.push(')');
            }
            Outcome::Described(None) => out.push_str(" :status ok :bound #f"),
            Outcome::Described(Some(d)) => {
                out.push_str(" :status ok :bound #t :kind ");
                push_string(&mut out, &d.kind);
                out.push_str(" :value ");
                push_string(&mut out, &d.value);
                if let Some(sig) = &d.signature {
                    out.push_str(" :signature ");
                    push_string(&mut out, sig);
                }
                if let Some(doc) = &d.doc {
                    out.push_str(" :doc ");
                    push_string(&mut out, doc);
                }
            }
            Outcome::Framed(f) => {
                out.push_str(" :status ok :framing ");
                out.push_str(match f {
                    Framing::Complete => "complete",
                    Framing::Incomplete => "incomplete",
                    Framing::Invalid => "invalid",
                });
            }
            Outcome::Pong { version } => {
                out.push_str(" :status ok :version ");
                push_string(&mut out, version);
            }
            Outcome::Malformed(why) => {
                out.push_str(" :status malformed :message ");
                push_string(&mut out, why);
            }
        }
        out.push(')');
        out
    }
}

/// A Scheme string literal. Hand-rolled rather than borrowed from the writer
/// because a response is built without an `Engine`.
fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn proper_list(e: &crate::Engine, mut v: crate::Value) -> Option<Vec<crate::Value>> {
    let mut out = Vec::new();
    loop {
        match v {
            crate::Value::Null => return Some(out),
            crate::Value::Obj(r) => match e.obj(r) {
                crate::Obj::Pair(car, cdr) => {
                    out.push(*car);
                    v = *cdr;
                }
                _ => return None,
            },
            _ => return None,
        }
    }
}

/// A symbol or a string as text, so `:op eval` and `:op "eval"` both work --
/// an Emacs client writes the first and a Python one the second.
fn text_of(e: &crate::Engine, v: crate::Value) -> Option<String> {
    match v {
        crate::Value::Symbol(s) => Some(e.interner.sym_name(s).to_string()),
        crate::Value::Keyword(k) => Some(e.interner.kw_name(k).to_string()),
        crate::Value::Obj(r) => match e.obj(r) {
            crate::Obj::Str(chars) => Some(chars.iter().collect()),
            _ => None,
        },
        _ => None,
    }
}
