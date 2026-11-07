"""ppmdiff.py A.ppm B.ppm -- per-channel differences between two PPMs (P3 or P6)."""
import sys


def read_ppm(path):
    data = open(path, "rb").read()
    tokens, i = [], 0
    while len(tokens) < 4:
        while data[i:i + 1].isspace():
            i += 1
        j = i
        while not data[j:j + 1].isspace():
            j += 1
        tokens.append(data[i:j].decode())
        i = j
    magic, w, h, _ = tokens[0], int(tokens[1]), int(tokens[2]), tokens[3]
    if magic == "P6":
        px = list(data[i + 1:i + 1 + 3 * w * h])
    else:
        px = [int(t) for t in data[i:].split()]
    assert len(px) == 3 * w * h, (path, len(px), 3 * w * h)
    return w, h, px


wa, ha, a = read_ppm(sys.argv[1])
wb, hb, b = read_ppm(sys.argv[2])
assert (wa, ha) == (wb, hb), "sizes differ"
diffs = [abs(x - y) for x, y in zip(a, b)]
hist = {}
for d in diffs:
    hist[d] = hist.get(d, 0) + 1
n = len(diffs)
print(f"{wa}x{ha}: {n} channels, max diff {max(diffs)}, mean {sum(diffs) / n:.4f}")
for d in sorted(hist)[:6]:
    print(f"  diff {d}: {hist[d]} ({100 * hist[d] / n:.3f}%)")
big = sum(c for d, c in hist.items() if d > 2)
print(f"  > 2 levels: {big} ({100 * big / n:.4f}%)")
if big:
    w = wa
    worst = sorted(range(n), key=lambda k: -diffs[k])[:5]
    for k in worst:
        p = k // 3
        print(f"  worst at x={p % w} y={p // w} ch={k % 3}: {a[k]} vs {b[k]}")
