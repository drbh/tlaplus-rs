# Model playground

Edit TSL or TLA+ on the left; inspect traces, states, and generated TLA+ on the
right. Inspired by the editor/workspace arrangement in `build-a-board/playground`.
This prototype uses the existing native model checker, with structured results.

## Run locally

Requires Rust/Cargo and Node 24+ (tested with Node 25).

```sh
cd playground
npm ci
npm run dev
```

Open http://127.0.0.1:7821. `dev` builds the Rust worker first. Each check starts a
fresh worker; Stop terminates it. Checks run automatically after an 800 ms pause
in editing. Cmd/Ctrl+Enter checks immediately. The default
example is the Die Hard puzzle: `NotSolved` intentionally fails to reveal its
six-move solution. Counter is a passing example; Unfair retry shows a repeating
liveness counterexample.

TSL declares its own configuration. TLA+ accepts either an embedded CONFIG
section or the separate configuration editor, which takes precedence when
nonempty. Language buttons keep drafts during the session; they do not translate
arbitrary source. After running TSL, **Generated TLA+ → Open as editable TLA+**
opens the actual compiled model and its configuration.

Source is saved in this browser. Share copies a URL with the source and config in
its fragment; it creates no hosted record. Opening a shared link loads and automatically checks its model. An edit marks previous results stale and disables source links
until the next run.

## Read results

- **Trace:** actual counterexample steps, action names, original TSL action
  locations, and changes to nested values. A temporal trace explicitly identifies
  its loop-back state. Temporal edges currently have a generic label.
- **State graph:** recorded transitions, including cycles and self loops, laid
  out by discovery depth. Click a state to inspect values and outgoing edges.
  Dashed nodes were not fully expanded. A failed or limited exploration is
  labeled partial. Symmetry-reduced graphs are labeled as quotients.
- **Generated TLA+:** inspect the compiler's real output and configuration.

The inspector compares trace states with their predecessor, or graph states
with their discovery parent. Counts refer to exploration, not necessarily to
trace length. State, time, and display limits never produce a passing verdict.

## Boundaries

This is a **trusted local development tool**, not a public execution service or
a static-only application. It binds to loopback and rejects cross-origin run
requests. Processes are disposable, but are not security sandboxes and have no
OS memory limit. Do not expose the development server to untrusted users.

Runs are limited to 15 seconds and at most 2,000 states; the server accepts two
concurrent runs. Source plus configuration is limited to 256 KB, requests to
300 KB, and returned JSON to 16 MB. The diagram draws at most 250 states and
1,200 edges, separately from checker limits. Single-file models and bundled
standard TLA+ modules are supported; sibling file imports are not yet supported.
Source maps currently link TSL actions, not arbitrary native TLA+ operators.
The checker supports its native TLA+ subset, not every TLC feature.

```sh
npm run build                  # type-check + production client bundle
npm run preview                # serve bundle AND local checker adapter
npx playwright install chromium
npm test                       # with dev/preview running on 7821
cargo test --manifest-path ../Cargo.toml -p tla-playground
```

A static host serving `dist/` alone cannot run checks. See
[the design document](docs/design.md) for output semantics, architecture,
tradeoffs, and the route to an entirely browser-hosted version.

## Puzzle gallery

The example selector includes all nine models in
[the puzzle gallery](../examples/19-puzzle-gallery/README.md). Trace and graph
selection also update compact assignment, equation, board, bank, position, and
schedule views, alongside the generic value inspector. These views read actual
checker values and fall back when the expected schema is absent. Static logic
puzzles produce an initial assignment; planning puzzles produce action sequences.

## Paxos

The selector includes [correct and broken Paxos models](../examples/20-paxos/README.md)
with a 2,000-state preset. The correct model checks 1,696 states; the broken
proposer produces a ten-transition agreement violation. The state view shows
per-acceptor promises, historical votes, proposals, and promise responses.
These are bounded safety examples, without a liveness or unbounded-proof claim.

## Editing workflow

Models check automatically on opening and 800 ms after the last source,
configuration, or state-limit edit. Further edits cancel the running request
and its native checker process; stale responses cannot replace newer results.
Previous results remain labeled as outdated until the next result arrives.
Use **Auto** to pause automatic checking, **Stop** to cancel the current run,
or **Cmd/Ctrl+Enter** to check immediately. Background checks do not switch
mobile users away from the source editor.

Open **Examples** (Cmd/Ctrl+K) to search the model library. The editor includes
TSL and TLA+ lexical highlighting, nested TLA+ comments, search (Cmd/Ctrl+F),
line wrapping, font-size controls, and cursor position. Generated TLA+ uses
the same highlighting in a read-only editor. Highlighting is lexical, not
semantic type analysis; diagnostics still come from the compiler/checker.
