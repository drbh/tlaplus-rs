# Reliable delivery with composable definitions

[Run the model](ReliableDelivery.tsl):

```sh
cargo run -p tlaplus-cli -- examples/18-reliable-delivery/ReliableDelivery.tsl
```

Expected: **100 reachable states**, safety checks pass, and `AllComplete` passes.

Alice and Bob each submit one request. A request or acknowledgment can be lost;
a client retries until acknowledged. A delivery can leave the request available
for another delivery. The server must apply each request exactly once despite
these duplicates. Messages are a bounded set: at most one request and one
acknowledgment per client are pending. This models possible repeated delivery,
not an unbounded queue or arbitrary numbers of identical packets.

The example exercises the DSL's composition boundary:

- `Message` is a nominal struct with `peer` and `kind` fields.
- `init` explicitly initializes the network, server counts, and client flags.
- `Count` describes valid application counts, independently of enumeration.
- `applied_once` accepts and returns `Count`; violations report named contracts.
- `apply_once` is an atomic helper called by both delivery actions. The explicit
  `next` block excludes the helper from independently executable transitions.
- `acknowledged` reads current state; `abstract_delivered` projects concrete
  server state to a set of delivered peers.

The abstract initial set is empty. `RefinesDelivery` checks that each concrete
transition projects to an abstract step that adds at most one peer and removes
none, allowing internal network steps to stutter. This is an explicit finite
check of an initial mapping and step condition, not a general refinement-mapping
language or an unbounded proof. The empty initial projection follows from the
initializer assigning zero to every server count.

`AcksAreSound` and `MessagesAreSound` prohibit fabricated acknowledgments.
Nested `Map<Peer, Count>` state checking prevents counts outside zero or one.
Strong fairness for each client's retry, receive, and acknowledgment action is
an explicit delivery assumption: opportunities recurring forever must eventually
be taken. Arbitrary permanent message loss is incompatible with guaranteed
completion. Safety does not need those fairness assumptions.

Regression tests deliberately break the model:

- Removing retry fairness produces an `AllComplete` liveness counterexample.
- Removing calls to `apply_once` exposes a false acknowledgment.
- Replacing `applied_once`'s constant result with `n + 1` exposes duplicate
  application through its postcondition assertion.

Native tests cover these mutations. TLC independently checks the exported
passing model, the missing-fairness mutation, and the broken postcondition, along
with separate domain and atomic-composition examples:

```sh
JAVA=/absolute/path/to/java \
TLA2TOOLS_JAR=/absolute/path/to/tla2tools.jar \
cargo test -p tlaplus-cli --test tlc_dsl -- --ignored
```

These two clients are instances represented by indexed state and parameterized
actions. Parameterized library-module instances, payload enums, and pattern
matching remain outside the implemented language.
