"""Change in criterion's point estimate between two logs in logs/, per bench.

    python3 compare.py 19b484b:e9f73c3 19b484b:19b484b_again
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LOGS = os.path.join(HERE, os.environ.get("LOGS", "logs"))
UNIT = {"ns": 1e-3, "µs": 1.0, "us": 1.0, "ms": 1e3, "s": 1e6}


def parse(tag):
    out, name = {}, None
    for line in open(os.path.join(LOGS, tag + ".log"), encoding="utf-8"):
        s = line.strip()
        m = re.match(r"^([a-z_]+/[a-z_/0-9]+)\s+time:", s)
        if m:
            name, s = m.group(1), s[len(m.group(1)):].strip()
        elif re.match(r"^[a-z_]+/[a-z_/0-9]+$", s):
            name = s
            continue
        m = re.match(r"time:\s+\[\S+ \S+ (\S+) (\S+) \S+ \S+\]", s)
        if m and name:
            out[name] = float(m.group(1)) * UNIT[m.group(2)]
            name = None
    return out


pairs = [a.split(":") for a in sys.argv[1:]]
data = {t: parse(t) for p in pairs for t in p}
print("bench".ljust(38) + "".join(f"{a} -> {b}".rjust(30) for a, b in pairs))
for bench in data[pairs[0][0]]:
    row = bench.ljust(38)
    for a, b in pairs:
        x, y = data[a][bench], data[b][bench]
        row += f"{(y - x) / x * 100:+.1f}% ({x:.1f} -> {y:.1f} µs)".rjust(30)
    print(row)
