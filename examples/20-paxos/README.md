# Single-decree Paxos

[Paxos.tsl](Paxos.tsl) checks agreement with three acceptors, two competing
ballots, two proposed values, and intersecting two-acceptor quorums.
[BrokenPaxos.tsl](BrokenPaxos.tsl) changes one rule: the proposer ignores
previously accepted values reported by its quorum.

Choose either model in the playground. Their presets allow 2,000 states.
The correct model exhausts **1,696 states** and passes its safety invariants.
The broken model finds an agreement violation in **10 transitions**. Select
trace steps to inspect promises, proposals, and historical votes side by side.
The graph shows only its documented display limit; the checker explores more.
Both models can be exported to editable TLA+ from the playground.

## What the model captures

The model follows the prepare/promise and propose/accept rules described in
[Lamport’s Paxos Made Simple](https://lamport.azurewebsites.net/pubs/paxos-simple.pdf).
A proposer must inherit the value with the highest accepted ballot reported
by its promise quorum, or use its own preference if none was accepted.
Two matching acceptances at one ballot choose a value.

Prepare requests for both ballots are available initially. Actions deliver
messages in arbitrary order. Promise responses retain the accepted value at
the time of the response, and issued proposals remain fixed. Historical votes
are retained: later acceptances must not erase evidence that a value was chosen.

One broken execution first records B’s acceptance of Red at ballot 1. B reports
that acceptance when promising ballot 2, but the broken proposer picks Blue.
A and B then choose Blue at ballot 2; C’s delayed acceptance at ballot 1 joins
B’s earlier vote to choose Red as well. The inheritance rule matters even when
the earlier value has not yet reached a majority at the time of the promise.

## Scope and validation

This is bounded single-decree safety checking, not Multi-Paxos or an unbounded
proof. The model assumes a unique proposer per ballot and persistent acceptor
state. It does not explicitly model crash recovery, leadership, retransmission,
or a learner protocol. Messages remain available, with arbitrary delay or
omission; no fairness or eventual-decision claim is made.

Native tests independently count historical majorities in every explored state,
check that either value can be chosen in different executions, and confirm
that the correct higher ballot sometimes inherits Red. TLC checks the generated
TLA+ too and agrees on the correct state count and broken invariant.

```sh
cargo test -p tlaplus-playground --test paxos
JAVA=/path/to/java TLA2TOOLS_JAR=/path/to/tla2tools.jar \
  cargo test -p tlaplus-playground --test paxos -- --ignored
```
