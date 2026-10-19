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

An [experimental modern DSL](examples/17-modern-dsl/README.md) adds typed state,
atomic actions, nondeterministic choice, and temporal properties:

```bash
tla examples/17-modern-dsl/AtomicCounter.tsl
tla --export examples/17-modern-dsl/Retry.tsl /tmp/tsl-retry-export
```

It compiles to ordinary TLA+ and uses the same checker. The examples include
expected counterexamples and a weak-versus-strong fairness comparison.

Every original example now has a `.tsl` counterpart beside it, including the
Point library. The [comparison index](examples/README.md#modern-syntax-comparisons)
links all 18 pairs and explains representation differences.

[VS Code syntax highlighting](editors/vscode/README.md) is available as a local
extension for `.tsl` files.

[examples](examples) is a numbered path from a four state counter to liveness, fairness and modules. each spec says what it teaches and what to expect:

```bash
tla examples/01-counter/Counter.tla
```

## layout

|                                                        |                                                    |
| ------------------------------------------------------ | -------------------------------------------------- |
| `tla-cli`                                              | the `tla` binary: arguments and output             |
| `tla-dsl`                                              | experimental typed action DSL and TLA+ export       |
| `tla-checker`                                          | the checker as a library: load, bind, run, outcome |
| `tla-syntax`, `tla-parser`, `tla-modules`, `tla-lower` | front end: parsing, module loading, lowering to IR |
| `tla-ir`, `tla-values`, `tla-value-expr`               | expression representation and values               |
| `tla-eval`, `tla-library`, `tla-runtime`, `tla-random` | evaluation and the standard modules                |
| `tla-model`, `tla-engine`                              | configured models, state generation, exploration   |
| `tla-temporal`, `tla-props`                            | liveness                                           |
| `tla-disk`, `tla-observe`                              | disk backed exploration and tracing                |
| `tla-oracle`, `tla-bridge`, `tla-conformance`          | differential testing against TLC; the corpus runners are `tla-conformance`'s examples |
| `tla-stack-tests`                                      | whole-stack tests and the crate layering check     |

the layering between these groups is declared under `[workspace.metadata.architecture]`
in `Cargo.toml` and enforced by `tla-stack-tests/tests/architecture.rs`. every crate also keeps
a shape enforced by `tla-stack-tests/tests/shape.rs`: module docs, typed errors in every
signature, and no function over 170 lines. both run with `cargo test --workspace`.

## develop

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --no-deps -- -D warnings
```

## Web playground

The local [playground](playground/README.md) supports editing TSL and TLA+,
source-linked counterexample traces, changed-state inspection, and explored-state
graphs. Its [design document](playground/docs/design.md) covers the output model
and a path toward browser-only execution.
