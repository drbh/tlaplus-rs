---- MODULE Counter ----
(* lesson 1: a spec is a state machine.

   VARIABLE declares the state. Init says where it starts. Next says how one
   state becomes the next: x' is the value of x after the step. the checker
   explores every reachable state and checks each INVARIANT in all of them.

   the CONFIG section at the bottom is what TLC would read from Counter.cfg.

   run:    tla examples/01-counter/Counter.tla
   expect: 4 distinct states found (1 initial), search depth 4
           Safety checks passed                                              *)
EXTENDS Naturals

VARIABLE x

Init == x = 0

Next == x' = (x + 1) % 4

TypeOK == x \in 0..3
====
---- CONFIG Counter ----
INIT Init
NEXT Next
INVARIANT TypeOK
====
