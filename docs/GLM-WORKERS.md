# Running GLM workers on Haste

How session 3 (2026-10-13) ran up to eight GLM 5.3 workers in parallel, merging about twenty
of their branches in an afternoon, and what to keep doing. The workers are Claude Code background
sessions whose model is GLM 5.3 on the free e-INFRA CZ student API; the lead (an Opus session)
writes the briefs, reviews every branch and merges it.

## The setup

- **The `glm` tool** (a Claude Code skill on the developer's machine, not in this repo) launches
  workers and runs a local gate in front of the e-INFRA endpoint. The gate holds the API keys,
  sends every request through a per-key slot pool, and logs each request (worker, key,
  concurrency, status, latency, tokens, rate-limit headers; no prompt content) so limits can be
  measured. `glm status` shows workers and keys, `glm limits` summarises the log.
- **Two keys, four slots each, two to four workers per key.** Each worker is pinned to one key
  (`--key key1|key2`) and runs at maximum reasoning effort (`--effort max`).
- **One worker, one task, one worktree.** Each task gets its own branch and worktree under
  `$HASTE_WT/<branch>` (with a `cache` symlink to the main checkout's cache), and its own brief.
  Workers commit on their branch; they never push, merge, rebase or switch branches.

## Writing a brief

What made GLM succeed, in this order of importance:

1. **One small, fully specified step.** Name the files and line numbers to read, the exact API
   to add, the tests that prove it, and the commands that define "done". GLM finished
   everything shaped like this in 10-60 minutes. It stalled on open-ended investigations
   ("find out why"), unless the brief narrowed the question to a few concrete conditions.
2. **Say what it may edit.** One crate or a few named files; when a lead-owned file
   (`haste-core`, `main.rs`) needs a line, the brief authorises exactly that line.
3. **Give facts it cannot look up.** Workers have no web access. Put enum orders, protocol
   values and wiki numbers in the brief, and say where they come from (e.g. 26.3's
   `player_action` statuses read from the jar's enum: the wiki's older numbers are off by one).
4. **Point at the test server and say not to start or stop servers**, or say how to start a
   private one (`scripts/test-server.sh <name> <port> <seconds>`, which stops itself).
5. **Never let it open a window** (vanilla client) or read images.
6. **The shared rules block** at the end of every brief: read narrowly (Grep, then line ranges),
   cut command output with `tail`/`grep`, stay in scope, run `cargo fmt`, commit with plain
   messages and no attribution lines, report in about 20 lines with file:line and what was not
   verified, and stop and report when stuck.

## Permissions

Workers never wait for approval: nobody is there to give it. Write workers run in `dontAsk`
mode with a broad pre-approved list (cargo, `scripts/*`, `target/*`, git on their own branch,
read and shell helpers); anything else is refused at once and the worker adapts. Edits are
allowed only inside the worktree; push, checkout, reset, network tools and image reads are
refused. A brief can add task-specific commands.

## Reviewing a worker's branch

Treat every report as a draft. For each branch:

- read the diff; check `git status` is clean and the author is the noreply identity;
- run `cargo fmt --check`, `cargo test` and `cargo clippy --all-targets -- -D warnings` for the
  touched crates yourself (workers often skip `cargo fmt`);
- run `#[ignore]`d tests that need the cache when the worker could not;
- look at screenshots yourself: GLM cannot see images, and once described a picture full of
  floating blocks as "grounded";
- check claims that sound like general knowledge: a worker claimed cargo runs tests
  single-threaded and set environment variables in tests, which race and could have rewritten
  the user's real config file;
- send fixes back to the same worker while it still has the context (it is quick), or make a
  two-line fix yourself; then `scripts/check-no-personal.sh`, push, `gh pr create`, merge when
  CI passes.

## Rate limits (measured)

e-INFRA's LiteLLM proxy allows exactly 4 parallel requests per key; there is no per-IP or
per-machine limit (9 in flight from one machine over two keys worked). It frees a slot only a
moment after the response ends, longer when the upstream is busy: refilling a slot the instant a
request ends got 11 % refused, 1.5 s later 6 %, 3 s later none; eight busy workers with many
short requests got 34 % refused at 1.5 s. The gate therefore keeps a finished request's slot
counted for an adaptive grace period (1-5 s, grown by refusals, shrunk by clean requests;
refusals then fell to about 3 %) and silently retries the 429s that still happen, so workers
never see them. One worker can have two requests in flight, so plan two to three workers per
key.

## Context and speed

- **Images are the context killer.** One screenshot counted as 200-450k input tokens and stayed
  in the context; a worker with a 500k context spent up to 25 minutes per request and made no
  progress in two hours. Workers are blocked from reading images.
- **Restart a stuck worker fresh** with a precise brief rather than nursing a huge context: the
  floating-blocks fix took a fresh worker 10 minutes after the first one stalled for two hours.
- Workers drift into Chinese at very long contexts; ask for English.

## Keeping features connected to the game

Many tasks produce library code the client does not use yet (sound, particles, options). To
keep the game itself moving, regularly give one of the running workers an integration task
that wires finished pieces into the client (for example saved options into the camera and
input, block entity packets into the world, the HUD into the window), and keep the README's
completion list current: a feature counts as done only when it works in the game window.
