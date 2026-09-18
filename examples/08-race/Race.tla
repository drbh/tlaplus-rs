---- MODULE Race ----
(* lesson 8: concurrency.

   NumThreads threads each add one to a shared counter. in a real program the
   increment is two steps, read then write, so each thread keeps its own tmp
   and its own program counter: tmp[t] and pc[t] are functions from threads.

   Next says "some thread takes its next step". the checker tries every
   interleaving, and finds the one where both threads read 0 before either
   writes, so the counter ends at 1.

   run:    tla examples/08-race/Race.tla
   expect: Invariant Correct is violated.
           a 5 state trace: both threads read 0 (tmp = <<0, 0>>) before
           either writes, ending with counter = 1 and both threads Done.
           steps are labeled Thread, the disjunct of Next that produced them *)
EXTENDS Integers

CONSTANT NumThreads
Threads == 1..NumThreads

VARIABLES counter, tmp, pc
vars == <<counter, tmp, pc>>

Init ==
  /\ counter = 0
  /\ tmp = [t \in Threads |-> 0]
  /\ pc = [t \in Threads |-> "GetCounter"]

GetCounter(t) ==
  /\ pc[t] = "GetCounter"
  /\ tmp' = [tmp EXCEPT ![t] = counter]
  /\ pc' = [pc EXCEPT ![t] = "IncCounter"]
  /\ UNCHANGED counter

IncCounter(t) ==
  /\ pc[t] = "IncCounter"
  /\ counter' = tmp[t] + 1
  /\ pc' = [pc EXCEPT ![t] = "Done"]
  /\ UNCHANGED tmp

Thread(t) == GetCounter(t) \/ IncCounter(t)

AllDone == \A t \in Threads : pc[t] = "Done"

Terminating == AllDone /\ UNCHANGED vars

Next == (\E t \in Threads : Thread(t)) \/ Terminating

TypeOK ==
  /\ counter \in 0..NumThreads
  /\ tmp \in [Threads -> 0..NumThreads]
  /\ pc \in [Threads -> {"GetCounter", "IncCounter", "Done"}]

Correct == AllDone => counter = NumThreads
====
---- CONFIG Race ----
CONSTANT NumThreads = 2
INIT Init
NEXT Next
INVARIANT TypeOK
INVARIANT Correct
====
