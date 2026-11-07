#!/bin/sh
# examples/raytracer/bench.sh -- render the demo scene across N scm processes.
#
#     examples/raytracer/bench.sh [N] [WxH] [OUT.ppm]
#
# N defaults to every online CPU.  Process K renders rows K, K+N, K+2N, ...
# (striped, not banded, so the rows through the glass sphere's dispersion
# spread across workers -- raylisp's render-parallel does the same), and
# `paste` interleaves the stripes back into one image.
#
# Two times are reported.  "render" is the slowest worker's own render loop,
# the figure comparable to raylisp's render-parallel and to ray's timed render.
# "wall" adds process start, scene build and writing text, which neither of
# those counts.
#
# WORKER replaces the worker command, which is "$SCM raytracer.scm" by
# default; the leaf-compiled hybrid is
#     WORKER=target/release/examples/raytrace_hybrid examples/raytracer/bench.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
scm=${SCM:-$here/../../target/release/scm}
n=${1:-$(getconf _NPROCESSORS_ONLN)}
size=${2:-1280x720}
out=${3:-demo.ppm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

now() { "$scm" -e '(current-jiffy)'; }   # microseconds

start=$(now)
k=0
while [ "$k" -lt "$n" ]; do
  # shellcheck disable=SC2086
  ${WORKER:-$scm $here/raytracer.scm} --size "$size" --stripe "$k/$n" \
    --out "$work/$k.txt" > "$work/$k.log" 2>&1 &
  k=$((k + 1))
done
wait
end=$(now)

# A worker with fewer rows than another leaves an empty line in paste's output.
files=$(k=0; while [ "$k" -lt "$n" ]; do printf '%s ' "$work/$k.txt"; k=$((k + 1)); done)
{
  printf 'P3\n%s %s\n255\n' "${size%x*}" "${size#*x}"
  # shellcheck disable=SC2086
  paste -d '\n' $files | grep -v '^$'
} > "$out"

render=$(sed -n 's/.* in \([0-9.]*\) s,.*/\1/p' "$work"/*.log | sort -n | tail -1)
pixels=$(( ${size%x*} * ${size#*x} ))
awk -v n="$n" -v px="$pixels" -v r="$render" -v w="$(( end - start ))" -v out="$out" 'BEGIN {
  printf "%d processes, %d px\n", n, px
  printf "  render %.3f s  %.2f Mpix/s  (slowest worker)\n", r, px / r / 1e6
  printf "  wall   %.3f s  %.2f Mpix/s  (with start-up and output)\n", w / 1e6, px / (w / 1e6) / 1e6
  printf "  -> %s\n", out
}'
