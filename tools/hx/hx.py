"""hx: codebase tools for agents working on Haste (and for people). Every command prints a
small, capped answer instead of raw search output, so a worker's context stays small.

  hx map [PATH]                Haste crates, or the files under PATH with sizes and summaries
  hx outline FILE              a Rust or vanilla Java file's items with line ranges
  hx def NAME                  where a Rust item is defined
  hx refs NAME [PATH]          uses of a name in Haste, grouped by file
  hx ported NAME               Haste code that cites a vanilla Class or Class.method
  hx vanilla CLASS[.method]... vanilla class outline (--grep RE filters it, --fields adds fields)
                               or a method's body with line numbers (every overload)
  hx vgrep [-l] RE [DIR]       search the vanilla source; each hit labelled Class.method:line
  hx check [CRATE...] [--no-tests] [--filter NAME]
                               cargo fmt, file sizes, clippy -D warnings and tests of the crates
                               changed since the branch base (or those given), errors only
  hx client --user NAME [--script FILE] [--walk FILE] [--seconds N] [--headless]
            [--before 'RCON CMD']... [--at 'SECONDS:RCON CMD']...
                               run target/release/haste against the test server, RCON before
                               and during the run, print a digest of the log and new screenshots
  hx done                      what is still missing before a worker may finish
  hx context BRIEF             the orientation pack for a brief (the worker launcher adds it)
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import checks  # noqa: E402
import client  # noqa: E402
import context  # noqa: E402
import guard  # noqa: E402
import haste  # noqa: E402
import vanilla  # noqa: E402


def main(argv):
    if not argv or argv[0] in ("-h", "--help", "help"):
        print(__doc__.strip())
        return 0
    cmd, args = argv[0], argv[1:]
    flags = {a for a in args if a.startswith("--")}
    pos = [a for a in args if not a.startswith("--")]

    def opt(name):
        if name in args:
            i = args.index(name)
            if i + 1 < len(args):
                pos.remove(args[i + 1])
                return args[i + 1]
        return None

    if cmd == "map":
        haste.cmd_map(pos[0] if pos else None)
    elif cmd == "outline" and pos:
        haste.cmd_outline(pos[0])
    elif cmd == "def" and pos:
        haste.cmd_def(pos[0])
    elif cmd == "refs" and pos:
        haste.cmd_refs(pos[0], pos[1] if len(pos) > 1 else "crates")
    elif cmd == "ported" and pos:
        haste.cmd_ported(pos[0])
    elif cmd == "vanilla" and pos:
        grep = opt("--grep")
        for name in pos:
            vanilla.cmd_class(name, grep=grep, fields="--fields" in flags)
    elif cmd == "vgrep":
        files_only = "-l" in pos
        pos = [p for p in pos if p != "-l"]
        if not pos:
            print("usage: hx vgrep [-l] REGEX [DIR under the source root]")
            return 2
        vanilla.cmd_vgrep(pos[0], pos[1] if len(pos) > 1 else None, files_only=files_only)
    elif cmd == "check":
        filt = opt("--filter")
        return 0 if checks.cmd_check(pos, skip_tests="--no-tests" in flags, filt=filt) else 1
    elif cmd == "client":
        def many(name):
            vals = []
            while name in args:
                i = args.index(name)
                vals.append(args[i + 1])
                del args[i:i + 2]
            return vals
        before, at = many("--before"), many("--at")
        user, script, walk = opt("--user"), opt("--script"), opt("--walk")
        seconds = int(opt("--seconds") or 60)
        if not user:
            print("usage: hx client --user NAME [--script FILE] [--walk FILE] [--seconds N] ...")
            return 2
        return 0 if client.cmd_client(user, script, walk, seconds, "--headless" in flags,
                                      before, at) else 1
    elif cmd == "done":
        return 0 if checks.cmd_done() else 1
    elif cmd == "context" and pos:
        context.cmd_context(pos[0])
    elif cmd == "hook" and pos:
        guard.main(pos[0])
    else:
        print(__doc__.strip())
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
