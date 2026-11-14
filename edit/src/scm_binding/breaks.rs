//! **The error pane's snapshot** (docs/editor-crate-plan.md, E3f) — SLIME's
//! sldb, as far as an editor whose sends run on the UI thread can take it.
//!
//! scm calls a BREAK HANDLER on an error nothing will catch, with the frames
//! still live (`scm-dap` serves its debugger from one). `scm-dap` can WAIT
//! in it, because its VM has a thread of its own; here a send runs on the
//! UI thread and winit has no nested event loop to wait in. So the handler
//! takes a snapshot — each frame's call, where it was written, and its
//! locals, printed — and aborts, and the shell shows the snapshot after the
//! send has failed. Evaluating in a frame, or continuing with a value, needs
//! a break that can resume later or a VM off the UI thread: scm's question,
//! not this crate's.
//!
//! **Only a send breaks.** The handler is installed for the whole session
//! but ARMED only by `%call-with-break`, around one send's thunk, and the
//! thunk runs in a nested evaluation: the editor's own `guard` around a send
//! is below that evaluation's base, so to scm the send's error is uncaught
//! and the break fires — with the send's frames, not the key dispatch's. An
//! error the sent code catches itself never breaks.

use std::cell::{Cell, RefCell};

use scm::Val;

/// The frames kept, and the locals per frame: a runaway recursion is
/// thousands deep, and the pane is for reading.
const MAX_FRAMES: usize = 40;
const MAX_LOCALS: usize = 24;
/// A local's printed value, cut to what a line can show.
const MAX_VALUE: usize = 160;

struct Frame {
    what: String,
    loc: Option<String>,
    locals: Vec<(String, String)>,
}

struct Break {
    message: String,
    frames: Vec<Frame>,
}

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static LAST: RefCell<Option<Break>> = const { RefCell::new(None) };
}

/// The global the shell reads the failed send's condition from — the very
/// object a `guard` around the send would have caught, so the error line
/// the transcript prints is the one it always printed.
const CONDITION: &str = "%break-condition";

fn on_break(vm: &mut scm::Vm, err: &scm::VmError) -> scm::BreakAction {
    // One snapshot per send, the innermost: a host prim that calls back
    // into Scheme breaks first at ITS evaluation's base, where the locals
    // of the failing code still are.
    if !ARMED.with(|a| a.replace(false)) {
        return scm::BreakAction::Abort;
    }
    let frames = vm
        .live_frames()
        .into_iter()
        .take(MAX_FRAMES)
        .map(|f| Frame {
            what: f.what,
            loc: f.loc,
            locals: vm
                .bindings_of(f.env)
                .into_iter()
                .take(MAX_LOCALS)
                .map(|(name, v)| (name, cut(scm::writer::pretty_str(&vm.engine, v, 70))))
                .collect(),
        })
        .collect();
    LAST.with(|l| *l.borrow_mut() = Some(Break { message: err.to_string(), frames }));
    // What the condition IS, as scm's own catch builds it: a raised object
    // as itself, a refusal from Rust as an error object.
    let condition = match err {
        scm::VmError::Raised(v, _) => *v,
        other => {
            let message = vm.engine.string(&other.to_string());
            vm.engine.error_obj(message, scm::Value::Null, scm::heap::ErrorKind::Machine)
        }
    };
    let sym = vm.engine.interner.symbol(CONDITION);
    vm.set_global(sym, condition);
    scm::BreakAction::Abort
}

fn cut(s: String) -> String {
    let flat: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if flat.chars().count() <= MAX_VALUE {
        flat
    } else {
        flat.chars().take(MAX_VALUE - 1).chain(std::iter::once('…')).collect()
    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    interp.vm_mut().set_break_handler(Some(Box::new(on_break)));
    interp.eval(&format!("(define {CONDITION} #f)"))?;

    // `(%call-with-break thunk)` — call THUNK with the break armed: #t when
    // it returned, #f when it raised (its value is the thunk's to keep —
    // a value crossing back here would be copied, and a send's value must
    // stay the object it is). After #f, `%last-break` describes the frames
    // and `%break-condition` holds what was raised.
    interp.define_fn_with("%call-with-break", |host, args| {
        let Some(Val::Proc(thunk)) = args.first() else {
            return Err("expected a procedure of no arguments".into());
        };
        LAST.with(|l| *l.borrow_mut() = None);
        ARMED.with(|a| a.set(true));
        let out = host.call(thunk, &[]);
        ARMED.with(|a| a.set(false));
        match out {
            Ok(_) => Ok(Val::Bool(true)),
            Err(message) => {
                // An error that did not break (a read error is raised before
                // any frame exists) still says what it was.
                LAST.with(|l| {
                    l.borrow_mut().get_or_insert_with(|| Break { message, frames: Vec::new() });
                });
                Ok(Val::Bool(false))
            }
        }
    })?;

    // `(%last-break)` — the last send that raised, as
    // `(message ((call location ((name printed) …)) …))`, innermost frame
    // first, location #f for a form built at run time; #f before any.
    interp.define_fn("%last-break", |_args| {
        Ok(LAST.with(|l| match &*l.borrow() {
            None => Val::Bool(false),
            Some(b) => Val::List(vec![
                Val::Str(b.message.clone()),
                Val::List(
                    b.frames
                        .iter()
                        .map(|f| {
                            Val::List(vec![
                                Val::Str(f.what.clone()),
                                f.loc.clone().map_or(Val::Bool(false), Val::Str),
                                Val::List(
                                    f.locals
                                        .iter()
                                        .map(|(n, v)| Val::List(vec![Val::Str(n.clone()), Val::Str(v.clone())]))
                                        .collect(),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ]),
        }))
    })?;
    Ok(vec!["%call-with-break", "%last-break"])
}
