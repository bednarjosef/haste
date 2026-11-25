"""The decompiled 26.3 client (cache/source/26.3/client): find classes, outline them, print
methods, and search with every hit labelled by the method it sits in.

The decompiler's layout is regular (three spaces per nesting level, a member's closing brace
alone on a line at the member's own indent), so members are found by indentation, not parsed.
Nothing from the source is ever written into the repository: this only prints it.
"""
import os
import re
import subprocess

from common import CACHE, clip, out

SOURCE = os.path.join(CACHE, "source", "26.3", "client")

# A member declaration: modifiers, a type or nothing (constructors), a name, an open paren.
MEMBER = re.compile(
    r"^(?P<ind> +)(?:@\w+(?:\([^)]*\))? +)*(?P<mods>(?:(?:public|protected|private|static|"
    r"final|abstract|synchronized|native|default|strictfp) +)*)(?:<[^>]*> +)?"
    r"(?:(?P<type>[\w.$<>\[\]?, @]+?) +)?(?P<name>\w+)\((?P<args>.*)$")
TYPE_DECL = re.compile(
    r"^(?P<ind> *)(?:@\w+(?:\([^)]*\))? +)*(?:(?:public|protected|private|static|final|"
    r"abstract|sealed|non-sealed|strictfp) +)*(?P<kind>class|interface|enum|record|@interface)"
    r" +(?P<name>\w+)")
FIELD = re.compile(
    r"^(?P<ind> +)(?:(?:public|protected|private|static|final|volatile|transient) +)+"
    r"(?P<type>[\w.$<>\[\]?, @]+?) +(?P<name>\w+)(?: = (?P<value>.*?))?;$")
KEYWORDS = {"if", "for", "while", "switch", "catch", "return", "new", "else", "try", "synchronized",
            "throw", "super", "this", "assert"}

_index = None


def index():
    """Simple class name -> list of paths relative to SOURCE (top-level classes only)."""
    global _index
    if _index is None:
        _index = {}
        for d, _, files in os.walk(SOURCE):
            for f in files:
                if f.endswith(".java"):
                    rel = os.path.relpath(os.path.join(d, f), SOURCE)
                    _index.setdefault(f[:-5], []).append(rel)
    return _index


def resolve(name):
    """'LivingEntity', 'Brain.Packed' or 'net/minecraft/x/Foo' -> (relpath, inner chain) or None."""
    if not os.path.isdir(SOURCE):
        raise SystemExit(f"no decompiled source at {SOURCE}: run scripts/decompile.sh")
    parts = name.replace("/", ".").split(".")
    # A full package path: take the longest prefix that is a file.
    for i in range(len(parts), 0, -1):
        rel = os.path.join(*parts[:i]) + ".java"
        if os.path.isfile(os.path.join(SOURCE, rel)):
            return [rel], parts[i:]
    hits = index().get(parts[0])
    if not hits:
        return None, parts[1:]
    return sorted(hits, key=lambda p: (p.count("/"), p)), parts[1:]


def read_lines(rel):
    with open(os.path.join(SOURCE, rel), encoding="utf-8", errors="replace") as f:
        return f.read().split("\n")


def block_end(lines, start, ind):
    """Index of the line closing the member that starts at `start` (indent `ind`)."""
    first = lines[start].rstrip()
    if first.endswith(";") and "{" not in first:
        return start
    close = " " * len(ind) + "}"
    for i in range(start + 1, len(lines)):
        s = lines[i].rstrip()
        if s == close or s.startswith(close + " ") or s == close + ";":
            return i
        if i == start + 1 and s.endswith(";") and "{" not in first and "{" not in s:
            return i  # a two-line abstract declaration
    return len(lines) - 1


def members(lines):
    """Every type, method and field: (kind, depth, name, first line, last line, signature).
    A method or field counts only at its enclosing type's member indent, so statements inside
    bodies (`new Foo(...)`, calls) are never taken for members."""
    types = []
    for i, line in enumerate(lines):
        m = TYPE_DECL.match(line)
        if m and len(m["ind"]) % 3 == 0:
            types.append(("type", len(m["ind"]) // 3, m["name"], i,
                          block_end(lines, i, m["ind"]), line.strip().rstrip("{").strip()))
    member_indent = {}
    for t in types:
        for i in range(t[3] + 1, t[4]):
            member_indent[i] = 3 * (t[1] + 1)
    res = list(types)
    starts = {t[3] for t in types}
    for i, line in enumerate(lines):
        if i in starts or i not in member_indent:
            continue
        want = member_indent[i]
        if len(line) - len(line.lstrip(" ")) != want:
            continue
        m = MEMBER.match(line)
        if m and m["name"] not in KEYWORDS and (m["type"] or "") not in KEYWORDS and (
                m["mods"] or m["type"]):
            res.append(("method", want // 3 - 1, m["name"], i, block_end(lines, i, m["ind"]),
                        short_sig(line)))
            continue
        m = FIELD.match(line)
        if m:
            res.append(("field", want // 3 - 1, m["name"], i, i, line.strip()))
    res.sort(key=lambda r: r[3])
    return res


def short_sig(line):
    s = line.strip().rstrip("{").strip()
    return re.sub(r"\bfinal ", "", s)


def owner_chain(mems, line_no):
    """'Outer.Inner.method' for a 0-based line, from the members that contain it."""
    chain = []
    for kind, _, name, a, b, _ in mems:
        if a <= line_no <= b and kind in ("type", "method"):
            chain.append(name)
    return ".".join(chain)


def pick_type(mems, lines, inner):
    """The (first, last) range of a nested type chain, or the whole file."""
    lo, hi = 0, len(lines) - 1
    for name in inner:
        found = [m for m in mems if m[0] == "type" and m[2] == name and lo <= m[3] <= hi]
        if not found:
            return None
        lo, hi = found[0][3], found[0][4]
    return lo, hi


EXTENDS = re.compile(r"\b(?:extends|implements) +([\w.]+(?:<[^{]*?>)?(?:, *[\w.]+(?:<[^{]*?>)?)*)")


def supertypes(sig):
    """Simple names of the classes and interfaces a type declaration extends."""
    names = []
    for m in EXTENDS.finditer(sig):
        for part in re.sub(r"<[^<>]*(?:<[^<>]*>[^<>]*)*>", "", m.group(1)).split(","):
            part = part.strip().split(".")[-1]
            if part:
                names.append(part)
    return names


def find_method(cls, method, inner=(), depth=0):
    """[(relpath, lines, members, member)] for `method` declared in `cls` or, when it is not,
    in the nearest supertype that declares it."""
    if depth > 8:
        return []
    paths, _ = resolve(cls)
    for rel in paths or []:
        lines = read_lines(rel)
        mems = members(lines)
        rng = pick_type(mems, lines, list(inner))
        if rng is None:
            continue
        found = [m for m in mems if m[0] == "method" and m[2] == method and rng[0] <= m[3] <= rng[1]]
        if found:
            return [(rel, lines, mems, m) for m in found]
        want = inner[-1] if inner else os.path.basename(rel)[:-5]
        decl = next((m for m in mems if m[0] == "type" and m[2] == want
                     and rng[0] <= m[3] <= rng[1]), None)
        for sup in supertypes(decl[5] if decl else ""):
            res = find_method(sup, method, (), depth + 1)
            if res:
                return res
    return []


def cmd_class(name, grep=None, fields=False):
    paths, rest = resolve(name)
    if not paths:
        out(f"no vanilla class '{name}' (try: hx vgrep 'class {name.split('.')[0]}')")
        return
    if len(paths) > 1:
        out(f"'{name}' is ambiguous: " + ", ".join(p[:-5] for p in paths[:8]) + "; using the first")
    rel = paths[0]
    lines = read_lines(rel)
    mems = members(lines)
    inner = [p for p in rest if p[:1].isupper()]
    method = next((p for p in rest if not p[:1].isupper()), None)
    rng = pick_type(mems, lines, inner)
    if rng is None:
        out(f"no nested type {'.'.join(inner)} in {rel}")
        return
    lo, hi = rng
    if method:
        found = find_method(name.split(".")[0], method, inner)
        if not found:
            cmd_method(rel, lines, mems, lo, hi, method)  # prints the near names
            return
        for frel, flines, fmems, m in found:
            if frel != rel:
                out(f"(not declared in {name.split('.')[0]}; inherited from {frel[:-5]})")
            print_member(frel, flines, fmems, m)
        return
    out(f"{rel}  ({len(lines)} lines)  outline: L<first>-<last>; print a method with "
        f"hx vanilla {name}.<method>")
    depth0 = len(inner)
    shown = 0
    for kind, depth, mname, a, b, sig in mems:
        if not (lo <= a <= hi) or depth < depth0 or depth > depth0 + 1:
            continue
        if kind == "field" and not fields and "static final" not in sig:
            continue
        if grep and not re.search(grep, sig, re.I):
            continue
        pad = "  " * (depth - depth0)
        rng_s = f"L{a + 1}" if a == b else f"L{a + 1}-{b + 1}"
        out(f"{rng_s:>12} {pad}{clip(sig, 150)}")
        shown += 1
    if not shown:
        out("  (nothing matched)")


def cmd_method(rel, lines, mems, lo, hi, method, max_lines=400):
    found = [m for m in mems if m[0] == "method" and m[2] == method and lo <= m[3] <= hi]
    if not found:
        near = sorted({m[2] for m in mems if m[0] == "method" and method.lower() in m[2].lower()})
        out(f"no method {method} in {rel}" + (f"; similar: {', '.join(near[:10])}" if near else ""))
        return
    for m in found:
        print_member(rel, lines, mems, m, max_lines)


def print_member(rel, lines, mems, m, max_lines=400):
    a, b = m[3], m[4]
    out(f"--- {rel}:{a + 1}-{b + 1}  {owner_chain(mems, a)}")
    for i in range(a, min(b, a + max_lines) + 1):
        out(f"{i + 1:5} {lines[i]}")
    if b - a > max_lines:
        out(f"  ... {b - a - max_lines} more lines: Read {rel} with offset {a + max_lines + 1}")


def cmd_vgrep(pattern, sub=None, limit=40, files_only=False):
    root = os.path.join(SOURCE, sub) if sub else SOURCE
    try:
        res = subprocess.run(["rg", "-n", "--no-heading", "-e", pattern, root],
                             capture_output=True, text=True)
    except FileNotFoundError:
        res = subprocess.run(["grep", "-rnE", pattern, root], capture_output=True, text=True)
    hits = [l for l in res.stdout.split("\n") if l]
    if not hits:
        out(f"no match for /{pattern}/ in the vanilla source" + (f" under {sub}" if sub else ""))
        return
    by_file = {}
    for h in hits:
        path, ln, text = h.split(":", 2)
        by_file.setdefault(os.path.relpath(path, SOURCE), []).append((int(ln), text))
    out(f"{len(hits)} hits in {len(by_file)} files" + (
        f" (showing {limit}; narrow the pattern or pass a package)" if len(hits) > limit else ""))
    if files_only:
        for rel, hs in sorted(by_file.items(), key=lambda kv: -len(kv[1]))[:limit]:
            out(f"{len(hs):4}  {rel[:-5]}")
        return
    shown = 0
    for rel, hs in sorted(by_file.items(), key=lambda kv: kv[0]):
        if shown >= limit:
            return
        mems = members(read_lines(rel))
        cls = os.path.basename(rel)[:-5]
        out(f"{rel}")
        for ln, text in hs:
            if shown >= limit:
                return
            owner = owner_chain(mems, ln - 1) or cls
            out(f"  {owner}:{ln}  {clip(text.strip(), 130)}")
            shown += 1
