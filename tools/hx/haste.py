"""Haste's own tree: the crate map, file outlines, definitions, uses, and which Haste code
ports a given vanilla class or method.

rustfmt is enforced, so an item's closing brace sits alone at the item's indent; items are found
by indentation, not parsed.
"""
import os
import re

from common import ROOT, clip, out, rg

ITEM = re.compile(
    r"^(?P<ind> *)(?:pub(?:\([^)]*\))? +)?(?:(?:async|const|unsafe|extern \"C\") +)*"
    r"(?P<kind>fn|struct|enum|trait|impl|mod|const|static|type|macro_rules!)\b(?P<rest>.*)$")


def crate_desc(crate_dir):
    try:
        for line in open(os.path.join(crate_dir, "Cargo.toml")):
            if line.startswith("description"):
                return line.split("=", 1)[1].strip().strip('"')
    except OSError:
        pass
    return ""


def module_doc(path):
    """First sentence of a file's //! header."""
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            doc = []
            for line in f:
                s = line.strip()
                if s.startswith("//!"):
                    doc.append(s[3:].strip())
                elif doc or (s and not s.startswith("//") and not s.startswith("#!")):
                    break
    except OSError:
        return ""
    text = " ".join(doc)
    m = re.match(r"(.+?[.:])(\s|$)", text)
    return (m.group(1) if m else text).strip()


def count_lines(path):
    with open(path, "rb") as f:
        return sum(1 for _ in f)


def cmd_map(path=None):
    if not path:
        out("Haste crates (hx map crates/<name> lists a crate's files):")
        base = os.path.join(ROOT, "crates")
        for c in sorted(os.listdir(base)):
            d = os.path.join(base, c)
            files = [os.path.join(r, f) for r, _, fs in os.walk(os.path.join(d, "src")) for f in fs
                     if f.endswith(".rs")]
            lines = sum(count_lines(f) for f in files)
            out(f"  {c:16} {len(files):4} files {lines:7} lines  {clip(crate_desc(d), 110)}")
        return
    full = os.path.join(ROOT, path)
    if os.path.isfile(full):
        cmd_outline(path)
        return
    out(f"{path}: lines, then the //! summary (hx outline FILE lists a file's items)")
    for r, dirs, fs in sorted(os.walk(full)):
        dirs[:] = sorted(d for d in dirs if d not in ("target", ".git"))
        for f in sorted(fs):
            if not f.endswith((".rs", ".java", ".wgsl")):
                continue
            p = os.path.join(r, f)
            rel = os.path.relpath(p, full)
            doc = module_doc(p) if f.endswith(".rs") else ""
            out(f"  {count_lines(p):5}  {rel:46} {clip(doc, 100)}")


def items(lines):
    """(kind, indent, first line, last line, signature, doc) for every Rust item."""
    res = []
    for i, line in enumerate(lines):
        m = ITEM.match(line)
        if not m or line.lstrip().startswith("//"):
            continue
        ind = m["ind"]
        end = i
        sig_lines = [line.strip()]
        if not line.rstrip().endswith(";"):
            close = ind + "}"
            for j in range(i + 1, len(lines)):
                s = lines[j].rstrip()
                if len(sig_lines) < 6 and "{" not in "".join(sig_lines):
                    sig_lines.append(s.strip())
                if s == close or s.startswith(close + " ") or s in (close + ";", close + ","):
                    end = j
                    break
                if j == i + 1 and s.endswith(";") and "{" not in line and m["kind"] != "impl":
                    end = j
                    break
        sig = " ".join(sig_lines).split("{")[0].strip().rstrip(";")
        doc = ""
        k = i - 1
        while k >= 0 and lines[k].strip().startswith(("#[", "///")):
            if lines[k].strip().startswith("///"):
                doc = lines[k].strip()[3:].strip()
            k -= 1
        res.append((m["kind"], len(ind), i, end, sig, doc))
    return res


def cmd_outline(path):
    full = os.path.join(ROOT, path) if not os.path.isabs(path) else path
    if path.endswith(".java") and "cache/source" in full:
        import vanilla
        vanilla.cmd_class(os.path.relpath(full, vanilla.SOURCE)[:-5])
        return
    with open(full, encoding="utf-8", errors="replace") as f:
        lines = f.read().split("\n")
    out(f"{path}  ({len(lines)} lines)  {clip(module_doc(full), 120)}")
    for kind, ind, a, b, sig, doc in items(lines):
        if ind > 8:
            continue
        rng = f"L{a + 1}" if a == b else f"L{a + 1}-{b + 1}"
        tail = f"  // {clip(doc, 70)}" if doc else ""
        out(f"{rng:>12} {' ' * (ind // 2)}{clip(sig, 120)}{tail}")


def cmd_def(name):
    pat = (rf"^\s*(pub(\([^)]*\))?\s+)?((async|const|unsafe)\s+)*"
           rf"(fn|struct|enum|trait|type|const|static|mod)\s+{re.escape(name)}\b"
           rf"|macro_rules!\s*{re.escape(name)}\b")
    hits = rg(["-n", "--no-heading", "-e", pat, "crates", "tools"])
    if not hits:
        hits = rg(["-n", "--no-heading", "-e", rf"impl\b.*\b{re.escape(name)}\b", "crates"])
        if not hits:
            out(f"no definition of {name} in crates/ (hx refs {name} finds its uses)")
            return
        out(f"no item named {name}; impl blocks mentioning it:")
    for h in hits[:30]:
        path, ln, text = h.split(":", 2)
        out(f"{path}:{ln}  {clip(text.strip(), 150)}")


def cmd_refs(name, path="crates", limit=40):
    hits = rg(["-n", "--no-heading", "-w", "-F", name, path])
    if not hits:
        out(f"no use of {name} under {path}")
        return
    by_file = {}
    for h in hits:
        p, ln, text = h.split(":", 2)
        by_file.setdefault(p, []).append((ln, text.strip()))
    out(f"{len(hits)} uses in {len(by_file)} files" + (
        f" (first {limit} lines; pass a narrower path)" if len(hits) > limit else ""))
    shown = 0
    for p, hs in sorted(by_file.items(), key=lambda kv: -len(kv[1])):
        out(f"{p}  ({len(hs)})")
        for ln, text in hs[:4]:
            if shown >= limit:
                return
            out(f"  {ln:>5}  {clip(text, 130)}")
            shown += 1


def cmd_ported(name, limit=30):
    """Haste files whose comments name a vanilla class or Class.method."""
    hits = rg(["-n", "--no-heading", "-e", rf"//.*\b{re.escape(name)}\b", "crates"])
    if not hits and "." in name:
        cls, meth = name.rsplit(".", 1)
        hits = rg(["-n", "--no-heading", "-e", rf"//.*\b{re.escape(meth)}\b", "crates"])
        hits = [h for h in hits if cls in open(os.path.join(ROOT, h.split(":", 1)[0])).read()]
    if not hits:
        out(f"no Haste comment names {name}: probably not ported yet")
        return
    by_file = {}
    for h in hits:
        p, ln, text = h.split(":", 2)
        by_file.setdefault(p, []).append((ln, text.strip()))
    out(f"Haste code citing {name}: {len(by_file)} files")
    for p, hs in sorted(by_file.items(), key=lambda kv: -len(kv[1]))[:limit]:
        ln, text = hs[0]
        out(f"  {p}:{ln}  ({len(hs)})  {clip(text, 110)}")
