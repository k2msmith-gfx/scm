//! S1's exit criterion, in two halves:
//!
//!   1. what `write` prints reads back `equal?` to what it was given, and
//!   2. a truncated datum reports *incomplete*, never *invalid*.
//!
//! The corpus half of the criterion -- every saved ray scene through
//! `Form::to_scheme()` -- needs ray's emitter to grow a Scheme dialect first.
//! What stands in for it until then is the scene shapes the ray-steel POC
//! actually produced, which is the closest thing to a real scene that exists.

use scm::{Engine, ReadError, Reader, ReaderOptions, Value, write_str};

/// Read every datum, print it, read it again, and insist the two agree.
#[track_caller]
fn round_trips(src: &str) {
    let mut e = Engine::new();
    let first = Reader::read_all(src, &mut e).unwrap_or_else(|err| panic!("read {src:?}: {err}"));
    let printed: Vec<String> = first.iter().map(|&v| write_str(&e, v)).collect();
    let joined = printed.join("\n");
    let second = Reader::read_all(&joined, &mut e)
        .unwrap_or_else(|err| panic!("re-read {joined:?}: {err}"));
    assert_eq!(
        first.len(),
        second.len(),
        "datum count changed: {src:?} -> {joined:?}"
    );
    for (i, (&a, &b)) in first.iter().zip(second.iter()).enumerate() {
        assert!(
            e.equal(a, b),
            "datum {i} of {src:?} printed as {:?} and read back as {:?}",
            write_str(&e, a),
            write_str(&e, b)
        );
    }
}

#[test]
fn atoms_round_trip() {
    for src in [
        "1", "-1", "0", "9223372036854775807", "-9223372036854775808",
        "1.5", "-2.5e-7", ".5", "-0.0", "1e300",
        "#t", "#f", "#true", "#false",
        "#\\a", "#\\space", "#\\newline", "#\\x41", "#\\(",
        "\"\"", "\"hi\"", "\"a\\nb\"", "\"quote \\\" inside\"", "\"back \\\\ slash\"",
        "sym", "+", "-", "...", "->", "a->b", "list?", "set!", "char<=?",
        "+inf.0", "-inf.0",
        ":at", ":look-at", ":aces",
        "()",
    ] {
        round_trips(src);
    }
}

#[test]
fn an_integral_flonum_stays_a_flonum() {
    // The trap: Rust prints 1.0f64 as "1", which reads back as an *exact*
    // integer. A scene whose camera moved from 1.0 to 1 would still render,
    // which is exactly the kind of silent drift the round trip exists to stop.
    let mut e = Engine::new();
    let v = scm::read_one("1.0", &mut e).unwrap().unwrap();
    assert!(matches!(v, Value::Flonum(_)), "1.0 read as {v:?}");
    assert_eq!(write_str(&e, v), "1.0");
    round_trips("1.0 -3.0 1e10");
}

#[test]
fn structures_round_trip() {
    for src in [
        "(1 2 3)",
        "(1 . 2)",
        "(1 2 . 3)",
        "(a (b (c)))",
        "#(1 2 3)",
        "[1 2 3]",
        "[[1 2] [3 4]]",
        "#u8(0 1 255)",
        "'a",
        "''a",
        "`(a ,b ,@c)",
        "(quote a b)", // three elements: must NOT print as 'a b
        "(a . (b . (c . ())))",
    ] {
        round_trips(src);
    }
}

#[test]
fn brackets_and_hash_vectors_stay_distinct() {
    // Not decoration: `[a b c]` evaluates its elements and `#(a b c)` does
    // not, so a writer that printed one as the other would change what a
    // saved scene means (plan §4.5).
    let mut e = Engine::new();
    let brack = scm::read_one("[1 2 3]", &mut e).unwrap().unwrap();
    let hash = scm::read_one("#(1 2 3)", &mut e).unwrap().unwrap();
    assert_eq!(write_str(&e, brack), "[1 2 3]");
    assert_eq!(write_str(&e, hash), "#(1 2 3)");
    // Same elements, so `equal?` -- the flag is about evaluation, not identity.
    assert!(e.equal(brack, hash));
}

#[test]
fn keywords_are_not_symbols() {
    let mut e = Engine::new();
    let kw = scm::read_one(":at", &mut e).unwrap().unwrap();
    let sym = scm::read_one("at", &mut e).unwrap().unwrap();
    assert!(matches!(kw, Value::Keyword(_)), ":at read as {kw:?}");
    assert!(matches!(sym, Value::Symbol(_)), "at read as {sym:?}");
    assert!(!e.equal(kw, sym), ":at and 'at must not be equal?");
    assert_eq!(write_str(&e, kw), ":at");

    // Interning: the same spelling is the same object.
    let again = scm::read_one(":at", &mut e).unwrap().unwrap();
    assert_eq!(kw, again);
}

#[test]
fn hash_colon_keywords_are_read_but_never_written() {
    // Reversed 2026-10-12 (plan §4.9). An earlier decision made `#:at` a
    // reader error, on the grounds that keywords are not in R7RS so no
    // portable library uses that spelling. True of libraries, and wrong about
    // the consumer that matters: a model writing keyword arguments emits
    // Racket's and Guile's `#:at` from habit, and LLM-generated scripts are a
    // stated requirement. So it is accepted on input -- and `write` still only
    // ever emits `:at`, so there is one canonical output.
    let mut e = Engine::new();
    let a = scm::read_one("#:at", &mut e).unwrap().unwrap();
    let b = scm::read_one(":at", &mut e).unwrap().unwrap();
    assert!(matches!(a, Value::Keyword(_)), "#:at read as {a:?}");
    assert_eq!(a, b, "#:at and :at must be the same keyword");
    assert_eq!(write_str(&e, a), ":at");

    // `#:` with nothing after it is still wrong.
    let err = scm::read_one("#: ", &mut e).unwrap_err();
    assert!(matches!(err, ReadError::Invalid { .. }), "got {err:?}");
}

#[test]
fn the_conformance_reader_turns_keywords_off() {
    // How `r7rs-tests.scm` will run: `:at` is a legal R7RS symbol, so a suite
    // that uses one must be able to get a symbol.
    let mut e = Engine::new();
    let opts = ReaderOptions {
        colon_keywords: false,
    };
    let v = Reader::with_options(":at", opts)
        .read(&mut e)
        .unwrap()
        .unwrap();
    assert!(matches!(v, Value::Symbol(_)), ":at read as {v:?}");
}

#[test]
fn comments_are_atmosphere() {
    round_trips("; a line comment\n(1 2)");
    round_trips("#| a block #| nested |# comment |# (1 2)");
    round_trips("(1 #;(this is skipped) 2)");
    let mut e = Engine::new();
    let vs = Reader::read_all("(1 #;2 3)", &mut e).unwrap();
    assert_eq!(write_str(&e, vs[0]), "(1 3)");
}

// ---- incomplete vs invalid ----------------------------------------------

#[track_caller]
fn assert_incomplete(src: &str) {
    let mut e = Engine::new();
    match scm::read_one(src, &mut e) {
        Err(ReadError::Incomplete) => {}
        other => panic!("{src:?} should be Incomplete, got {other:?}"),
    }
}

#[track_caller]
fn assert_invalid(src: &str) {
    let mut e = Engine::new();
    match scm::read_one(src, &mut e) {
        Err(ReadError::Invalid { .. }) => {}
        other => panic!("{src:?} should be Invalid, got {other:?}"),
    }
}

#[test]
fn truncation_is_incomplete_not_invalid() {
    // Every one of these is "keep typing", and a REPL that said "syntax
    // error" to any of them would be unusable.
    for src in [
        "(",
        "(1 2",
        "(1 (2",
        "[1 2",
        "#(1 2",
        "#u8(1",
        "\"unterminated",
        "\"trailing escape \\",
        "|unterminated",
        "#| unclosed",
        "#| nested #| inner |#",
        "'",
        "`",
        ",",
        "(1 .",
        "#;",
        "#\\",
    ] {
        assert_incomplete(src);
    }
}

#[test]
fn wrongness_is_invalid_not_incomplete() {
    for src in [
        ")",
        "]",
        "(. 1)",
        "(1 . 2 3)",
        "#\\nosuchname",
        "#q",
        "#u9(1)",
        "\"bad \\q escape\"",
        "#u8(256)",
        "#u8(-1)",
    ] {
        assert_invalid(src);
    }
}

#[test]
fn an_unsupported_number_says_so() {
    // No `ratios`, no bignums (plan §2.3). Both must be errors rather than
    // quietly becoming something else -- Steel's `(/ 8 3)` silently turning
    // into bignum arithmetic is the cautionary tale.
    let mut e = Engine::new();
    let err = scm::read_one("1/2", &mut e).unwrap_err();
    assert!(format!("{err}").contains("ratios"), "got {err}");

    let err = scm::read_one("99999999999999999999999999", &mut e).unwrap_err();
    assert!(format!("{err}").contains("64-bit"), "got {err}");
}

#[test]
fn radix_and_exactness_prefixes() {
    let mut e = Engine::new();
    for (src, want) in [
        ("#x1f", Value::Fixnum(31)),
        ("#o17", Value::Fixnum(15)),
        ("#b1011", Value::Fixnum(11)),
        ("#d42", Value::Fixnum(42)),
        ("#x-ff", Value::Fixnum(-255)),
        ("#i42", Value::Flonum(42.0)),
        ("#e42.0", Value::Fixnum(42)),
    ] {
        let got = scm::read_one(src, &mut e).unwrap().unwrap();
        assert_eq!(got, want, "{src}");
    }
}

// ---- the closest thing to a real scene ----------------------------------

#[test]
fn a_scene_in_the_shape_ray_writes() {
    // Lifted from the ray-steel POC's cornell-metals.scm, with brackets put
    // back -- the shape plan §4 is designed around, including a computed
    // bracket, which is why brackets are a constructor rather than a literal.
    let scene = r#"
;;; cornell-metals.scm
(define (matte rgb)
  (material :albedo rgb :shader :lambertian))

(define white [0.73 0.73 0.73])

(define (scene)
  (set-film 768 768)
  (set-camera :at [0 1 3.9] :look-at [0 1 0] :fov 39.0)
  (set-integrator :path)
  (set-samples 768)
  (set-sky [0 0 0] [0 0 0])
  (set-tone-map :reinhard)
  (add-cuboid :size [2.08 0.02 2.08] :at [0 -0.01 0]
              :material (matte white) :name :floor)
  (add-sphere :at [(- 0.58) 0.28 (* -1 0.35)] :radius 0.28
              :material (material :albedo [1.0 0.71 0.29] :metallic 1 :roughness 0.05)
              :name :polished))
(scene)
"#;
    round_trips(scene);

    // And the bracket really is a form, not a constant: its elements are
    // still unevaluated data here, but they are *there* as a call.
    let mut e = Engine::new();
    let vs = Reader::read_all("[(- 0.58) 2]", &mut e).unwrap();
    assert_eq!(write_str(&e, vs[0]), "[(- 0.58) 2]");
}

// ---- the question a REPL and an editor both ask -------------------------

#[test]
fn parse_status_separates_keep_typing_from_wrong() {
    use scm::{ParseStatus, parse_status};
    // Complete: evaluate it.
    assert_eq!(parse_status("(+ 1 2)"), ParseStatus::Complete);
    assert_eq!(parse_status("(+ 1 2) (+ 3 4)"), ParseStatus::Complete);
    assert_eq!(parse_status(""), ParseStatus::Complete);
    assert_eq!(parse_status("; just a comment\n"), ParseStatus::Complete);

    // Incomplete: print a continuation prompt. A REPL that said "syntax
    // error" to any of these would be unusable.
    assert_eq!(parse_status("(define (f n)"), ParseStatus::Incomplete);
    assert_eq!(parse_status("(let ([x 1]"), ParseStatus::Incomplete);
    assert_eq!(parse_status("\"unterminated"), ParseStatus::Incomplete);
    assert_eq!(parse_status("#| open"), ParseStatus::Incomplete);
    assert_eq!(parse_status("'"), ParseStatus::Incomplete);

    // Invalid: say so now rather than swallowing more input.
    assert!(matches!(parse_status(")"), ParseStatus::Invalid(_)));
    assert!(matches!(parse_status("(1 . 2 3)"), ParseStatus::Invalid(_)));
}

#[test]
fn a_slash_in_an_identifier_is_not_a_rational() {
    // `constant-pi/2`, `n/2` and `inexact-+1/2` are identifiers in the R7RS
    // benchmarks; the reader refused all three as unsupported rationals.
    for src in ["n/2", "constant-pi/2", "inexact-+1/2", "a/b", "/", "1/", "/2"] {
        round_trips(src);
    }
    // The real thing is still refused with the reason, not misread.
    let mut e = scm::Engine::new();
    for src in ["1/2", "-3/4", "+1/2"] {
        let err = scm::read_one(src, &mut e).unwrap_err().to_string();
        assert!(err.contains("ratios"), "{src}: {err}");
    }
}
