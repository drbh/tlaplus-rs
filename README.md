# tlaplus-rs

a model checker for TLA+ written in Rust. it gives the same answers as TLC (verdicts, reachable states, counterexamples) without copying its Java structure or exact output

## build

with Rust installed through rustup, or from the `nix develop` shell:

```bash
git clone https://github.com/drbh/tlaplus-rs.git
cd tlaplus-rs
cargo build --release -p tla-cli
export PATH="$PWD/target/release:$PATH"
```

## use

```bash
tla examples/16-puzzles/RiverCrossing.tla
```

```
9 distinct states found (1 initial), search depth 7

Invariant NotSolved is violated.

State 1: Initial predicate
  side = [farmer |-> "L", wolf |-> "L", goat |-> "L", cabbage |-> "L"]
...
```

the config is the sibling `.cfg`, or a `CONFIG` section at the end of the spec, unless `--config` says otherwise. `tla --help` lists the rest (`--workers`, `--no-deadlock`, `--max-states`, `--library`, `--disk`)

## learn

[examples](examples) is a numbered path from a four state counter to liveness, fairness and modules. each spec says what it teaches and what to expect:

```bash
tla examples/01-counter/Counter.tla
```

## layout

each workspace crate is a top level directory, at most 5 files and under 510 lines per file. the limits are enforced by the tests

|                                                                   |                                                    |
| ----------------------------------------------------------------- | -------------------------------------------------- |
| `tla-cli`                                                         | the `tla` binary                                   |
| `tla-syntax`, `tla-parser`, `tla-modules`, `tla-lower`            | front end: parsing, module loading, lowering to IR |
| `tla-ir`, `tla-values`, `tla-value-expr`                          | expression representation and values               |
| `tla-eval`, `tla-library`, `tla-runtime`, `tla-random`            | evaluation and the standard modules                |
| `tla-model`, `tla-constants`, `tla-engine`                        | configured models, state generation, exploration   |
| `tla-temporal`, `tla-behavior`, `tla-props`                       | liveness                                           |
| `tla-disk`, `tla-store`, `tla-observe`                            | disk backed exploration and tracing                |
| `tla-oracle`, `tla-bridge`, `tla-*-conformance*`, `tla-snapshots` | differential testing against TLC                   |


## develop

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --no-deps -- -D warnings
```
