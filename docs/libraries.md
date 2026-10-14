# Libraries: `define-library` and `import`

2026-10-13. `scm/src/vm/library.rs`, tests in `scm/tests/library.rs`.
Added for the general-purpose goal: R7RS code is written as libraries, and
the SRFIs that make a Scheme useful for more than one project arrive as
libraries.

## The model

**A library is an environment frame.** Its body runs with that frame as
the environment, so its `define`s land in the frame and never in the
global namespace. A `define-syntax` inside it binds the macro as a value in
the frame -- the mechanism `let-syntax` already used -- so library macros
are scoped the same way. The same change made an internal `define-syntax`
in a procedure body scoped to that body, where before it leaked into the
global macro table.

**Importing copies bindings.** A library importing another gets slots in
its own frame; a program importing at top level -- the REPL, a file, a
`scm build` bundle -- gets globals, so user code stays on the fast T1
global path. `only`, `except`, `prefix` and `rename` are rewrites of that
copy and nest as R7RS specifies. `(scheme base)` and the other standard
names are the prelude, so importing them is a no-op.

**Where libraries come from.** `(acme util)` is looked for as
`acme/util.sld`, then `.scm`, in the current directory, on
`SCM_LIBRARY_PATH` (colon-separated), on paths given with
`Vm::add_library_path`, and finally in the set embedded in the binary,
which is how a bundle has them too. A library file holds `define-library`
forms only. `include` resolves relative to the library's own file.
`cond-expand` works in declarations and at top level, with `and`, `or`,
`not`, `(library …)` and the names `(features)` reports.

**What runs where.** A library body interprets on T0 at load time, which
is right for definitions; its procedures compile on T1 at their first call
like everything else. A macro exported from a library that expands to a
reference to one of the library's *private* bindings compiles too: the
library's frame is permanent, so the compiler reads the slot by address
(`Op::Slot`) rather than declining, and the test shows a hot loop over such
a macro running on T1 with no unit declined.

## The one simplification

A binding is copied at import, so a library that `set!`s its own exported
variable *after* it was imported does not reach its importers. R7RS makes
`set!` on an imported binding an error, and no library in the wild does
the other thing. It can become a shared cell later without changing any
program; it is recorded so that it is a decision and not a surprise.

## Errors, named

`library not found: (srfi 99) (looked for srfi/99.sld on . and
SCM_LIBRARY_PATH)`; `only: `nope` is not exported`; `export: `x` is not
defined in the library`; `library (a) imports itself, through (a)`;
`define-library: only at top level`.

## Agreement

Every test runs the same program under T0 and T1 and asserts the same
answer, and the library frames survive GC stress. The differential fuzzer
does not yet generate libraries; that is the next thing to teach it.
