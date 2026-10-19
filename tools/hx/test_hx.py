"""Tests for hx's parsers and hooks on small synthetic sources (no cache needed).

  python3 -m unittest discover -s tools/hx
"""
import io
import json
import os
import sys
import unittest
from contextlib import redirect_stdout

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import guard  # noqa: E402
import haste  # noqa: E402
import vanilla  # noqa: E402

JAVA = """package a;

public class Outer extends Base implements Thing<String>, Other {
   private static final int LIMIT = 30;
   private int count;

   public Outer(final int count) {
      this.count = count;
   }

   public void tick() {
      if (this.count > 0) {
         new Helper(this.count);
      }
      Runnable r = () -> {
         this.count++;
      };
   }

   protected abstract int size();

   private static class Helper {
      Helper(int n) {
      }

      void run() {
      }
   }
}
""".split("\n")

RUST = """//! A module. Second sentence.
use std::fmt;

/// The thing.
pub struct Thing {
    a: u32,
}

impl Thing {
    /// Makes one.
    pub fn new(
        a: u32,
    ) -> Self {
        let s = format!("{}", a);
        Self { a }
    }
}

pub const MAX: u32 = 3;
""".split("\n")


class VanillaMembers(unittest.TestCase):
    def setUp(self):
        self.m = {(k, n): (a + 1, b + 1) for k, _, n, a, b, _ in vanilla.members(JAVA)}

    def test_methods_and_ranges(self):
        self.assertEqual(self.m[("method", "tick")], (11, 18))
        self.assertEqual(self.m[("method", "Outer")], (7, 9))
        self.assertEqual(self.m[("method", "size")], (20, 20))
        self.assertEqual(self.m[("type", "Helper")], (22, 28))
        self.assertEqual(self.m[("method", "run")], (26, 27))

    def test_statements_are_not_members(self):
        names = {n for _, n in self.m}
        self.assertNotIn("Helper", {n for k, n in self.m if k == "method" and n == "Helper"
                                    and self.m[(k, n)][0] == 13})
        self.assertNotIn("r", names)

    def test_fields_and_owner(self):
        self.assertIn(("field", "LIMIT"), self.m)
        mems = vanilla.members(JAVA)
        self.assertEqual(vanilla.owner_chain(mems, 15), "Outer.tick")
        self.assertEqual(vanilla.owner_chain(mems, 26), "Outer.Helper.run")

    def test_supertypes(self):
        decl = next(m for m in vanilla.members(JAVA) if m[2] == "Outer" and m[0] == "type")
        self.assertEqual(vanilla.supertypes(decl[5]), ["Base", "Thing", "Other"])


class HasteItems(unittest.TestCase):
    def test_items_and_docs(self):
        got = {(k, a + 1, b + 1, doc) for k, _, a, b, _, doc in haste.items(RUST)}
        self.assertIn(("struct", 5, 7, "The thing."), got)
        self.assertIn(("impl", 9, 17, ""), got)
        self.assertIn(("fn", 11, 16, "Makes one."), got)
        self.assertIn(("const", 19, 19, ""), got)

    def test_multiline_signature(self):
        fn = next(i for i in haste.items(RUST) if i[0] == "fn")
        self.assertEqual(fn[4], "pub fn new( a: u32, ) -> Self")


class Guard(unittest.TestCase):
    def decide(self, cmd, background=False):
        buf = io.StringIO()
        with redirect_stdout(buf):
            try:
                guard.pre({"tool_name": "Bash",
                           "tool_input": {"command": cmd, "run_in_background": background}})
            except SystemExit:
                pass
        text = buf.getvalue()
        return json.loads(text)["hookSpecificOutput"]["permissionDecision"] if text else "allow"

    def test_refusals(self):
        self.assertEqual(self.decide("sleep 1", background=True), "deny")
        self.assertEqual(self.decide("grep -rn tick cache/source/26.3/client"), "deny")
        self.assertEqual(self.decide("cargo test"), "deny")
        self.assertEqual(self.decide("cargo clippy --all-targets"), "deny")
        self.assertEqual(self.decide("for f in a b; do echo $f; done"), "deny")
        self.assertEqual(self.decide("echo $(date)"), "deny")
        self.assertEqual(self.decide("C=crates; ls $C"), "deny")
        self.assertEqual(self.decide("cat > notes.txt <<'EOF'"), "deny")
        self.assertEqual(self.decide("git -C ../x status"), "deny")
        self.assertEqual(self.decide("cd /tmp && git status"), "deny")
        self.assertEqual(self.decide(
            "hx client --user a --before 'summon x {CustomName:'\"'\"'\"S\"'\"'\"'}'"), "deny")

    def test_allowed(self):
        self.assertEqual(self.decide("grep -rln tick cache/source/26.3/client"), "allow")
        self.assertEqual(self.decide("rg tick cache/source | head -20"), "allow")
        self.assertEqual(self.decide("cargo test -p haste-entity"), "allow")
        self.assertEqual(self.decide("cat Cargo.toml"), "allow")
        self.assertEqual(self.decide("hx check"), "allow")
        self.assertEqual(self.decide("awk '{print $1}' Cargo.toml"), "allow")
        self.assertEqual(self.decide("cd crates && ls | head -2"), "allow")
        self.assertEqual(self.decide('grep -n "end$" Cargo.toml'), "allow")


class Commands(unittest.TestCase):
    def test_commands_file(self):
        import tempfile
        import client
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write("# setup\nbefore: op a\n5: summon armor_stand ~ ~ ~ {CustomName:'S'}\n")
        try:
            self.assertEqual(client.read_commands(f.name),
                             (["op a"], ["5.0:summon armor_stand ~ ~ ~ {CustomName:'S'}"]))
        finally:
            os.unlink(f.name)


class Checklist(unittest.TestCase):
    def test_open_items_block_done(self):
        import tempfile
        import checks
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, "target"))
            old = checks.ROOT
            checks.ROOT = d
            try:
                self.assertIn("no checklist", checks.checklist_missing()[0])
                with open(os.path.join(d, "target", "hx-checklist.md"), "w") as f:
                    f.write("- [x] pose\n- [ ] armour layer\n- [blocked: no server] screenshot\n")
                left = checks.checklist_missing()
                self.assertEqual(len(left), 1)
                self.assertIn("armour layer", left[0])
                with open(os.path.join(d, "target", "hx-checklist.md"), "w") as f:
                    f.write("- [x] pose\n- [x] armour layer\n")
                self.assertEqual(checks.checklist_missing(), [])
                with open(os.path.join(d, "target", "hx-checklist.md"), "a") as f:
                    f.write("- [blocked: not in the dump] armour\n")
                old_state = checks.state_file
                checks.state_file = lambda: os.path.join(d, "hx-check")
                try:
                    self.assertEqual(len(checks.unreviewed_blocked()), 1)
                    self.assertEqual(checks.unreviewed_blocked(), [])
                finally:
                    checks.state_file = old_state
            finally:
                checks.ROOT = old


if __name__ == "__main__":
    unittest.main()
