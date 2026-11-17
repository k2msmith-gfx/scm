#!/bin/sh
# edit against Neovim and Emacs on the same file, the same keys and the same text area
# (57 rows x 244 columns: a 1920x1080 window at 2x in Monaco 13). Neovim runs
# and Emacs (-nw) run in a hidden terminal of that size, so no window opens;
# nvim's -n keeps a killed run from leaving a swap file that stalls the next
# one at its prompt. Each is run with the user's config and bare.
#
#   edit/tools/bench/run.sh FILE.scm OUTDIR
set -eu
file=$1 out=$2 here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
cargo run --release -q -p edit --example bench_ops -- "$file" "$out/edit.txt" 1920 1080 2
for cfg in full clean; do
  cp "$file" "$out/file-$cfg.scm"
  if [ $cfg = clean ]; then set -- --clean -c "syntax on | set number relativenumber"; else set --; fi
  python3 -I "$here/ptyrun.py" 57 244 nvim -n "$@" --cmd "let g:bench_out='$out/vim-$cfg.txt'" \
    -S "$here/nvim_bench.vim" "$out/file-$cfg.scm"
done
cp "$file" "$out/file-doom.scm"
python3 -I "$here/ptyrun.py" 57 244 emacs -nw --eval "(setq bench-out \"$out/emacs-doom.txt\" bench-evil t bench-file \"$out/file-doom.scm\")" \
  "$out/file-doom.scm" -l "$here/emacs_bench.el"
cp "$file" "$out/file-emacs-q.scm"
python3 -I "$here/ptyrun.py" 57 244 emacs -Q -nw --eval "(progn (setq bench-out \"$out/emacs-q.txt\" bench-evil nil bench-file \"$out/file-emacs-q.scm\") (menu-bar-mode -1) (global-display-line-numbers-mode 1))" \
  "$out/file-emacs-q.scm" -l "$here/emacs_bench.el"
python3 -I - "$out" <<'PY'
import statistics as st, sys
out = sys.argv[1]
def load(f):
    return {p[0]: sorted(float(x) * 1000 for x in p[1:]) for p in (l.split() for l in open(f))
            if p[0] in ("scroll-j", "page-down", "insert", "search")}
runs = {"edit": load(f"{out}/edit.txt"), "nvim": load(f"{out}/vim-full.txt"), "nvim --clean": load(f"{out}/vim-clean.txt"),
        "emacs": load(f"{out}/emacs-doom.txt"), "emacs -Q": load(f"{out}/emacs-q.txt")}
for op in ("scroll-j", "page-down", "insert", "search"):
    print(op)
    for name, d in runs.items():
        xs = d[op]
        print(f"  {name:14s} median {st.median(xs):7.3f} ms  p95 {xs[int(.95 * len(xs)) - 1]:7.3f}  max {xs[-1]:7.3f}")
print("(check each run's OUT.text and its 'lines' line before reading the times)")
PY
