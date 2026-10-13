# spikes

Throwaway code kept for reproducibility, not for reuse. Nothing here is part
of the `scm` crate and nothing here should grow.

- **s0-llvm** — S0: inkwell builds IR, legacy MCJIT runs it, ORCv2/LLJIT via
  `llvm-sys::orc2` runs it, and a second module is added afterwards.
- **s0-leaf-survey** — S0: what an authored texel function and an authored
  field actually cost in ray. `texcost.janet` sweeps make-texture 128..2048
  and field-bake :res 48..128; `fb-build.janet` times fruit-bowl's scene build
  with no render. Run from ray's root with its janet-featured binary:
  `./target/release/ray --scene <file> --out /tmp/x.png --batch`.
  Results in [../docs/leaf-survey.md](../docs/leaf-survey.md).
- **s5b-leaf** — S5b: the leaf probe. The purity/reachability analysis over
  the IR as it exists, then codegen + LLJIT for ray's real texture stack, to
  learn what the representation makes hard before it hardens. The analysis
  half needs no LLVM; the codegen half is behind `--features llvm`, which
  re-proves the two-sided gate on a crate that also depends on `scm`.
  Results in [../docs/s5b-findings.md](../docs/s5b-findings.md).
- **s0-nollvm** — S0: the feature gate. Proves the default build needs no LLVM
  and that `--features jit` fails loudly without it.

Run them with LLVM 21 from nix:

    LLVM=$(nix build --no-link --print-out-paths 'nixpkgs#llvmPackages_21.llvm.dev')
    cd s0-llvm
    PATH="$LLVM/bin:$PATH" LLVM_SYS_211_PREFIX="$LLVM" cargo run --release

And the gate, which must succeed with no LLVM anywhere:

    cd s0-nollvm
    env -u LLVM_SYS_211_PREFIX PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo build --release
