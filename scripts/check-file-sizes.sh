#!/usr/bin/env bash
# File size guard (AGENTS.md "Structured like vanilla"): keeps Haste's files small and focused.
#
# Every tracked Rust, Java and WGSL file under crates/ and tools/ must stay within its limit:
#   600 lines for source files, 1000 lines for test files (tests.rs, tests/*.rs, *_tests.rs, and
#   the Java under src/test). Files that were already over the limit when the guard was added are
#   listed in scripts/large-files.txt with their size at that time: they may shrink but never grow.
# New code for a big file goes into a new module named after the vanilla concept it follows.
#
# usage: scripts/check-file-sizes.sh           check (exit 1 on any violation)
#        scripts/check-file-sizes.sh --update  after a split: lower the recorded sizes to the
#                                              current ones and drop files now within the limit
#                                              (never raises a size or adds a file)
set -uo pipefail
cd "$(dirname "$0")/.."
list=scripts/large-files.txt
declare -A recorded
while read -r path size; do
  [ -n "${path:-}" ] && [ "${path:0:1}" != "#" ] && recorded[$path]=$size
done < <(cat "$list" 2>/dev/null)

limit_for() {
  case "$1" in
    */tests.rs|*/tests/*|*_tests.rs|*/src/test/*) echo 1000 ;;
    *) echo 600 ;;
  esac
}

bad=0
declare -A now
while IFS= read -r f; do
  [ -f "$f" ] || continue
  n=$(wc -l <"$f")
  now[$f]=$n
  lim=$(limit_for "$f")
  rec=${recorded[$f]:-}
  if [ -n "$rec" ]; then
    if [ "$n" -gt "$rec" ]; then
      echo "$f: $n lines, grew past its recorded $rec (listed in $list: it may only shrink; put new code in a new module)"
      bad=1
    fi
  elif [ "$n" -gt "$lim" ]; then
    echo "$f: $n lines, over the $lim-line limit (split it along vanilla's concepts into new modules)"
    bad=1
  fi
done < <(git ls-files 'crates/*.rs' 'crates/*.java' 'crates/*.wgsl' 'tools/*.rs' 'tools/*.java')

if [ "${1:-}" = "--update" ]; then
  {
    echo "# Files over the size limit when scripts/check-file-sizes.sh was added, with their size then."
    echo "# They may shrink but never grow. Lowered automatically by: scripts/check-file-sizes.sh --update"
    for f in $(printf '%s\n' "${!recorded[@]}" | sort); do
      n=${now[$f]:-0}
      [ "$n" -gt "$(limit_for "$f")" ] || continue
      rec=${recorded[$f]}
      [ "$n" -lt "$rec" ] && rec=$n
      echo "$f $rec"
    done
  } >"$list.tmp" && mv "$list.tmp" "$list"
  echo "updated $list"
  exit 0
fi
exit $bad
