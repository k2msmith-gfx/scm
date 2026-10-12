# spikes

Throwaway code kept for reproducibility, not for reuse. Nothing here is part
of the `scm` crate and nothing here should grow.

- **s0-llvm** — S0: inkwell builds IR, legacy MCJIT runs it, ORCv2/LLJIT via
  `llvm-sys::orc2` runs it, and a second module is added afterwards.
- **s0-nollvm** — S0: the feature gate. Proves the default build needs no LLVM
  and that `--features jit` fails loudly without it.

Run them with LLVM 21 from nix:

    LLVM=$(nix build --no-link --print-out-paths 'nixpkgs#llvmPackages_21.llvm.dev')
    cd s0-llvm
    PATH="$LLVM/bin:$PATH" LLVM_SYS_211_PREFIX="$LLVM" cargo run --release

And the gate, which must succeed with no LLVM anywhere:

    cd s0-nollvm
    env -u LLVM_SYS_211_PREFIX PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo build --release
