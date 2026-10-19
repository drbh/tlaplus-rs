# Model playground

Edit TSL or TLA+ on the left; inspect traces, states, and generated TLA+ on the
right. Inspired by the editor/workspace arrangement in `build-a-board/playground`.
The hosted site at https://tla.dholtz.com runs the Rust model checker as WebAssembly
in a browser worker. Model source and results stay in the browser.

## Hosted build and deployment

GitHub Actions builds and tests the site on pull requests and deploys green pushes
on `main` to Cloudflare Workers at `tla.dholtz.com`. The workflow also supports a
manual run on `main`. It uses the `CLOUDFLARE_API_TOKEN` repository secret, with
Workers deployment and custom-domain permissions in the configured account.
The local `.env` is ignored and is never bundled into the site.

```sh
rustup target add wasm32-wasip1
cd playground
npm ci
npm run build:browser
npx vite preview --host 127.0.0.1
# In another terminal:
npm run test:browser
```

The build script downloads WASI SDK 34 for the C parser dependencies, or uses
`WASI_SDK_PATH` when set. It stages the checker dependency closure separately
from development-only oracle adapters that depend on a sibling checkout, while
retaining the root lockfile’s package versions. Cloudflare serves `dist/`, including the 8.4 MB checker.
Each check runs in a fresh worker with an in-memory filesystem; Stop and the
15-second deadline terminate that worker. The 2,000-state and 16 MB result
limits apply in both modes. Initial checker download/compilation is included in
the deadline. No native process or model-checking API is needed for hosting.

## Run locally with the native checker

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

## Native development server boundaries

The native dev/preview adapter is a **trusted local development tool**, not a public execution service or
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

The default `npm run build` uses the native adapter; use `npm run build:browser`
for a standalone static site.

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
