"""Token and cost accounting of worker runs from their stream-json event logs.

Totals (cost, tokens by type, exact output) come from each segment's final result event, which
covers every API call the session made. The stream's per-message usage is used only for request
counts and prompt sizes: it carries the first partial output count, so it under-reports output.
"Context filled by" weighs each tool result by its size times the requests that re-read it.

  python3 tools/worker/costs.py [RUNS_DIR] [NAME_FILTER]
"""
import collections
import glob
import json
import os
import re
import subprocess
import sys

WT = os.environ.get("HASTE_WT") or os.path.join(os.path.dirname(os.path.dirname(subprocess.run(
    ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], capture_output=True,
    text=True, cwd=os.path.dirname(os.path.abspath(__file__))).stdout.strip())), "haste-wt")


def tool_cat(name, inp):
    if name != "Bash":
        return name
    c = inp.get("command", "")
    for pat, cat in ((r"^\s*hx\b", "hx"), (r"\bcargo\b", "cargo"), (r"\b(grep|rg|find|ls)\b", "grep/find/ls"),
                     (r"\b(cat|sed -n|head|tail)\b", "cat/sed/head")):
        if re.search(pat, c):
            return "Bash: " + cat
    return "Bash: other"


def segment(path):
    reqs = collections.OrderedDict()
    pending, results, result = {}, [], None
    for line in open(path):
        try:
            o = json.loads(line)
        except ValueError:
            continue
        t = o.get("type")
        if t == "assistant":
            m = o["message"]
            reqs[m["id"]] = m.get("usage") or {}
            for b in m.get("content") or []:
                if b.get("type") == "tool_use":
                    pending[b["id"]] = tool_cat(b.get("name"), b.get("input") or {})
        elif t == "user":
            for b in (o.get("message") or {}).get("content") or []:
                if isinstance(b, dict) and b.get("type") == "tool_result":
                    c = b.get("content")
                    text = c if isinstance(c, str) else json.dumps(c)
                    results.append((len(reqs), pending.get(b.get("tool_use_id"), "?"), len(text)))
        elif t == "result":
            result = o
    prompts = [u.get("input_tokens", 0) + u.get("cache_read_input_tokens", 0)
               + u.get("cache_creation_input_tokens", 0) for u in reqs.values()]
    weights = collections.Counter()
    for ri, cat, chars in results:
        weights[cat] += chars / 4 * max(len(reqs) - ri, 0)
    return dict(prompts=prompts, weights=weights, result=result)


def main(d, filt=None):
    runs = collections.defaultdict(list)
    for f in sorted(glob.glob(os.path.join(d, "*.jsonl"))):
        name = re.sub(r"\.r\d+$", "", os.path.basename(f)[:-6])
        if not filt or filt in name:
            runs[name].append(f)
    for name, files in runs.items():
        segs = [segment(f) for f in files]
        prompts = [p for s in segs for p in s["prompts"]]
        weights = sum((s["weights"] for s in segs), collections.Counter())
        done = [s["result"] for s in segs if s["result"]]
        cost = sum(r.get("total_cost_usd", 0) for r in done)
        mu = collections.Counter()
        for r in done:
            for v in (r.get("modelUsage") or {}).values():
                mu.update({k: v.get(k, 0) for k in ("inputTokens", "outputTokens", "thinkingTokens",
                                                    "cacheReadInputTokens", "cacheCreationInputTokens")})
        state = done[-1]["subtype"] if len(done) == len(segs) else "running"
        mins = sum(r.get("duration_ms", 0) or 0 for r in done) / 60000
        print(f"== {name} ({len(files)} segment{'s' * (len(files) > 1)}): {state}, "
              f"${cost:.2f}, {len(prompts)} requests, {mins:.0f} min")
        if prompts:
            over = sum(p > 100_000 for p in prompts)
            print(f"   prompt: first {prompts[0]:,}  mean {sum(prompts) // len(prompts):,}  peak "
                  f"{max(prompts):,}  over 100K: {over}")
        if mu:
            print(f"   tokens: cache read {mu['cacheReadInputTokens']:,}  cache write "
                  f"{mu['cacheCreationInputTokens']:,}  input {mu['inputTokens']:,}  output "
                  f"{mu['outputTokens']:,} (thinking {mu['thinkingTokens']:,})")
        tw = sum(weights.values()) or 1
        print("   context filled by: " + ", ".join(
            f"{k} {100 * v / tw:.0f}%" for k, v in weights.most_common(5)))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else os.path.join(WT, "runs"),
         sys.argv[2] if len(sys.argv) > 2 else None)
