//! `write` and `display`.
//!
//! `write` has one job beyond being readable: **what it prints must read back
//! `equal?` to what it was given.** That is S1's exit criterion, and it is
//! also what makes a saved scene trustworthy, so the two flonum cases that
//! would quietly break it -- an integral float printing as `1`, and a vector
//! forgetting which brackets it was written with -- are handled deliberately
//! rather than incidentally.

use crate::heap::{Engine, Obj};
use crate::value::Value;

pub fn write_str(e: &Engine, v: Value) -> String {
    let mut out = String::new();
    print(e, v, true, &mut out);
    out
}

pub fn display_str(e: &Engine, v: Value) -> String {
    let mut out = String::new();
    print(e, v, false, &mut out);
    out
}

fn print(e: &Engine, v: Value, quoted: bool, out: &mut String) {
    match v {
        Value::Fixnum(n) => out.push_str(&n.to_string()),
        Value::Flonum(x) => out.push_str(&flonum(x)),
        Value::Bool(true) => out.push_str("#t"),
        Value::Bool(false) => out.push_str("#f"),
        Value::Char(c) => {
            if quoted {
                out.push_str(&character(c));
            } else {
                out.push(c);
            }
        }
        Value::Symbol(id) => out.push_str(&symbol(e.interner.sym_name(id))),
        Value::Keyword(id) => {
            out.push(':');
            out.push_str(e.interner.kw_name(id));
        }
        Value::Primitive(id) => {
            out.push_str("#<primitive ");
            out.push_str(e.prim_name(id));
            out.push('>');
        }
        Value::Null => out.push_str("()"),
        Value::Unspecified => out.push_str("#<unspecified>"),
        Value::Eof => out.push_str("#<eof>"),
        Value::Obj(r) => match e.obj(r) {
            Obj::Str(chars) => {
                if quoted {
                    out.push('"');
                    for &c in chars {
                        match c {
                            '"' => out.push_str("\\\""),
                            '\\' => out.push_str("\\\\"),
                            '\n' => out.push_str("\\n"),
                            '\t' => out.push_str("\\t"),
                            '\r' => out.push_str("\\r"),
                            c => out.push(c),
                        }
                    }
                    out.push('"');
                } else {
                    out.extend(chars.iter());
                }
            }
            Obj::Vector { items, bracket } => {
                // The flag is not decoration: `[1 2 3]` evaluates its elements
                // and `#(1 2 3)` does not, so printing one as the other would
                // change the meaning of a saved scene (plan §4.5).
                out.push_str(if *bracket { "[" } else { "#(" });
                for (i, &item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    print(e, item, quoted, out);
                }
                out.push(if *bracket { ']' } else { ')' });
            }
            Obj::Bytevector(bytes) => {
                out.push_str("#u8(");
                for (i, b) in bytes.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    out.push_str(&b.to_string());
                }
                out.push(')');
            }
            Obj::Pair(..) => pair(e, v, quoted, out),
            // None of these have datum syntax, so none of them round-trips.
            // Printing them at all is for the REPL and for error messages.
            Obj::Closure { name, .. } => match name {
                Some(n) => {
                    out.push_str("#<procedure ");
                    out.push_str(e.interner.sym_name(*n));
                    out.push('>');
                }
                None => out.push_str("#<procedure>"),
            },
            Obj::Env { .. } => out.push_str("#<environment>"),
            Obj::Parameter { .. } => out.push_str("#<parameter>"),
            Obj::Rng(_) => out.push_str("#<random-source>"),
            // Printed as the message it carries, so `(display e)` in a
            // handler reads the way a person would expect.
            Obj::Port { kind, .. } => {
                out.push_str(match kind {
                    crate::heap::PortKind::StringIn { .. } => "#<input-port string>",
                    crate::heap::PortKind::StringOut(_) => "#<output-port string>",
                    crate::heap::PortKind::Stdout => "#<output-port stdout>",
                    crate::heap::PortKind::Stderr => "#<output-port stderr>",
                });
            }
            Obj::ErrorObj {
                message, irritants, ..
            } => {
                print(e, *message, false, out);
                let mut cur = *irritants;
                while let Value::Obj(r2) = cur {
                    let Obj::Pair(car, cdr) = e.obj(r2) else { break };
                    out.push(' ');
                    print(e, *car, true, out);
                    cur = *cdr;
                }
            }
        },
    }
}

/// Lists, with the four abbreviations printed as abbreviations -- a saved
/// scene reads better with `'floor` than `(quote floor)`, and it reads back
/// the same either way.
fn pair(e: &Engine, v: Value, quoted: bool, out: &mut String) {
    if let Some((name, inner)) = abbreviation(e, v) {
        out.push_str(name);
        print(e, inner, quoted, out);
        return;
    }
    out.push('(');
    let mut cur = v;
    let mut first = true;
    loop {
        match cur {
            Value::Obj(r) => match e.obj(r) {
                Obj::Pair(car, cdr) => {
                    if !first {
                        out.push(' ');
                    }
                    print(e, *car, quoted, out);
                    first = false;
                    cur = *cdr;
                }
                _ => {
                    out.push_str(" . ");
                    print(e, cur, quoted, out);
                    break;
                }
            },
            Value::Null => break,
            other => {
                out.push_str(" . ");
                print(e, other, quoted, out);
                break;
            }
        }
    }
    out.push(')');
}

/// `(quote x)` -> `'x`, but only for exactly two elements: `(quote a b)` is an
/// ordinary three-element list and must print as one.
fn abbreviation(e: &Engine, v: Value) -> Option<(&'static str, Value)> {
    let Value::Obj(r) = v else { return None };
    let Obj::Pair(car, cdr) = e.obj(r) else {
        return None;
    };
    let Value::Symbol(sym) = car else { return None };
    let name = match e.interner.sym_name(*sym) {
        "quote" => "'",
        "quasiquote" => "`",
        "unquote" => ",",
        "unquote-splicing" => ",@",
        _ => return None,
    };
    let Value::Obj(r2) = cdr else { return None };
    let Obj::Pair(inner, Value::Null) = e.obj(*r2) else {
        return None;
    };
    Some((name, *inner))
}

/// Flonums must come back as flonums. Rust's shortest round-trip formatting
/// prints `1` for `1.0f64`, which would read back as the *exact* integer 1 --
/// so a decimal point is forced when nothing else marks it inexact.
fn flonum(x: f64) -> String {
    if x.is_nan() {
        return "+nan.0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "+inf.0".into() } else { "-inf.0".into() };
    }
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn character(c: char) -> String {
    match c {
        ' ' => "#\\space".into(),
        '\n' => "#\\newline".into(),
        '\t' => "#\\tab".into(),
        '\r' => "#\\return".into(),
        '\0' => "#\\null".into(),
        '\u{7}' => "#\\alarm".into(),
        '\u{8}' => "#\\backspace".into(),
        '\u{7f}' => "#\\delete".into(),
        '\u{1b}' => "#\\escape".into(),
        c if (c as u32) < 0x20 => format!("#\\x{:x}", c as u32),
        c => format!("#\\{c}"),
    }
}

/// A symbol needs `|…|` only if reading it back would not give the same
/// symbol -- which includes the empty symbol, anything with a delimiter in
/// it, and anything the number parser would claim.
fn symbol(name: &str) -> String {
    let needs_bars = name.is_empty()
        || name.chars().any(|c| {
            c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '"' | ';' | '\'' | '`' | ',' | '|' | '#')
        })
        || name.starts_with(':')
        || looks_like_number(name);
    if !needs_bars {
        return name.to_string();
    }
    let mut out = String::from("|");
    for c in name.chars() {
        match c {
            '|' => out.push_str("\\|"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('|');
    out
}

fn looks_like_number(name: &str) -> bool {
    // `+`, `-` and `...` are ordinary symbols; `1`, `-2.5`, `.5` are not.
    if matches!(name, "+" | "-" | "..." | "->") {
        return false;
    }
    let body = name.strip_prefix(['+', '-']).unwrap_or(name);
    body.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '.')
}
