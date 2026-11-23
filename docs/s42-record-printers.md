# S42: record printers

**Done 2026-11-22.** `(set-record-type-printer! type printer)`: a record
type's printed form, for `write`, `display`, `write-shared`,
`write-simple` and a host's printing. The language's statement is
`docs/subset.md`, "The additions", 7.

## Why

lux keeps an app's state in models, and a model is a record of one type,
`%mrec`, holding its layout and its values. scm printed any record by its
type's name, so every model printed `#<%mrec>` -- at the REPL, in edit's
transcript, in an error. The trial of lux against cl-wgpu (cl-wgpu
`docs/trial/README.md`, lux item 6) found its developer calling
`model->list` at every look. A printer for one type is lux's to write; the
hook for it is scm's, and is any consumer's.

## The spelling

Guile's (`(srfi srfi-9 gnu)`), fetched from its manual: "Scheme Syntax:
set-record-type-printer! type proc -- Where type corresponds to the first
argument of define-record-type, and proc is a procedure accepting two
arguments, the record to print, and an output port." No SRFI or R7RS
text covers how a record prints, so the nearest established name was taken
rather than a new one.

## How

The printer in `writer.rs` is Rust over the heap and cannot call Scheme --
it is called from ~40 places, error messages among them, with only the
engine in hand. So printing runs the printers first:

1. `%records-in` (Rust) walks the value as the printer would -- pairs and
   vectors, not into records -- and answers each record whose type has a
   printer, once.
2. The prelude runs each printer into a string port, with the record on
   `%printing` meanwhile: met again inside its own printing (a record that
   holds itself, or a cycle of them) it is left to print as `#<type>`.
3. `%set-record-views!` hands the texts to the engine
   (`Engine::record_views`), the print runs, and a `dynamic-wind` clears
   them, error or not.

`write`, `display`, `write-shared` and `write-simple` stay the primitives
until a program sets its first printer, which rebinds them to the
prelude's `%viewed-write` and the rest, over the primitives (`%write` and
the rest). So a program that sets none -- ray's -- runs exactly what it
ran before, and the leaf compiler still refuses `display` as an impure
primitive: the first version wrapped them always, and the LLVM gate's
`refusals_name_the_call_and_the_chain` caught the leaf compiler then
calling `display` a procedure with a rest parameter instead. `Interp::write`, `display`, `pretty` and
`describe`'s value do the same from Rust, calling `%record-views`; there a
printer that raises leaves its records as `#<type>`, since a REPL's answer
should not fail for a printer's sake.

Each record in the table is reachable from the value being printed, and
nothing allocates between setting the table and printing, so the
collector needs no part in it; the tests run again under GC stress.

## Tests

- `scm/tests/srfi/record-printer.scm`, on both tiers and under GC stress
  (`tests/srfi.rs`): lists, vectors, improper lists, nesting, a record
  that holds itself, a cycle of two, replacing, a failing printer leaving
  nothing behind, the current output port.
- `scm/tests/embed.rs`, `a_hosts_printing_uses_record_printers`: `write`,
  `display`, `pretty`, `describe`, and the fallback.
- `scm/tests/subset.rs`: the claims in `docs/subset.md`.
