//! S11a: live recompilation (plan §3.2a, consequence 3).
//!
//! Redefine a texture at the REPL while a render is running. The workers
//! hold a native function pointer, and LLJIT owns the code memory, so the
//! code cannot be pulled out from under them. S11 gave the handle a
//! lifetime -- a `Compiled` keeps its JIT alive until the last clone drops.
//! This adds the other half, ray's `live.rs` pattern: a **generation**
//! bumped on every replacement, so a pass in flight keeps the pointer it
//! snapshotted and the next pass picks up the new one, and a pass that
//! wants to abandon early can compare the generation between strips.
//!
//! Staleness is a comparison, not a guess. The VM bumps a version on every
//! store to a global; a `Live` records the versions of the leaf, every
//! helper it reaches and every table it captured at compile time, and
//! `refresh` recompiles only when one of them has moved. A refusal on
//! refresh keeps the old code running and reports -- the REPL shows the
//! message, the render goes on.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use scm::{SymId, Vm};

use crate::{Compiled, Refusal, Shape, compile, verdict};

struct Entry {
    compiled: Compiled,
    /// (global, its version when this code was compiled).
    deps: Vec<(SymId, u64)>,
}

pub struct Live {
    name: String,
    shape: Shape,
    current: RwLock<Entry>,
    generation: AtomicU64,
}

impl Live {
    /// Compile `name` and start watching its definition.
    pub fn compile(vm: &mut Vm, name: &str, shape: Shape) -> Result<Live, Refusal> {
        let entry = Self::build(vm, name, shape)?;
        Ok(Live {
            name: name.to_string(),
            shape,
            current: RwLock::new(entry),
            generation: AtomicU64::new(0),
        })
    }

    fn build(vm: &mut Vm, name: &str, shape: Shape) -> Result<Entry, Refusal> {
        let summary = verdict(vm, name)?;
        let compiled = compile(vm, name, shape)?;
        let deps = std::iter::once(name)
            .chain(summary.calls.iter().map(String::as_str))
            .chain(summary.captured.iter().map(String::as_str))
            .filter_map(|n| vm.engine.interner.lookup(n))
            .map(|s| (s, vm.global_version(s)))
            .collect();
        Ok(Entry { compiled, deps })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// The generation: bumped by every successful replacement.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// The code as of now, and the generation it belongs to. A pass takes
    /// this once at its start; the clone keeps that code alive for as long
    /// as the pass holds it, whatever `refresh` does meanwhile.
    pub fn snapshot(&self) -> (Compiled, u64) {
        let guard = self.current.read().unwrap();
        (guard.compiled.clone(), self.generation.load(Ordering::Acquire))
    }

    /// Has anything this leaf was compiled from been redefined since?
    pub fn stale(&self, vm: &Vm) -> bool {
        let guard = self.current.read().unwrap();
        guard.deps.iter().any(|&(s, v)| vm.global_version(s) != v)
    }

    /// Recompile if stale. `Ok(Some(generation))` when new code is in
    /// place, `Ok(None)` when nothing had changed, `Err` when the new
    /// definition is refused -- in which case the old code stays current
    /// and the deps are left stale, so the next `refresh` tries again.
    pub fn refresh(&self, vm: &mut Vm) -> Result<Option<u64>, Refusal> {
        if !self.stale(vm) {
            return Ok(None);
        }
        let entry = Self::build(vm, &self.name, self.shape)?;
        Ok(Some(self.install(entry)))
    }

    /// Unconditionally recompile -- after a `vector-set!` on a captured
    /// table, say, which moves no version.
    pub fn recompile(&self, vm: &mut Vm) -> Result<u64, Refusal> {
        let entry = Self::build(vm, &self.name, self.shape)?;
        Ok(self.install(entry))
    }

    fn install(&self, entry: Entry) -> u64 {
        let old = {
            let mut guard = self.current.write().unwrap();
            std::mem::replace(&mut *guard, entry)
        };
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        // The old code is dropped here -- or later, by whichever pass still
        // holds a clone of it.
        drop(old);
        generation
    }
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#<live {} {:?} gen {}>", self.name, self.shape, self.generation())
    }
}
