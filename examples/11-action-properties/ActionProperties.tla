---- MODULE ActionProperties ----
(* lesson 11: action properties.

   an invariant looks at one state. an action property looks at a step: a
   pair of states, with x the value before and x' the value after. "the
   counter never goes down" is not a fact about any single state, it is
   [][counter' >= counter]_counter.

   the brackets matter. TLA+ lets any step leave the variables unchanged
   (stuttering), so [A]_v means "A, or v did not change", and the checker
   only accepts action properties in that shape. a quantifier goes inside
   the box: [][\A t \in Threads : ...]_vars, not \A t : [][...]_vars.

   this is also the first spec with Spec == Init /\ [][Next]_vars, the usual
   way to write a full specification, and the config names it with
   SPECIFICATION instead of INIT and NEXT.

   run:    tla examples/11-action-properties/ActionProperties.tla
   expect: Safety checks passed

   try changing IncCounter to counter' = tmp[t] - 1: Monotonic fails on the
   step from 1 to 0, a step whose two states are each fine on their own    *)
EXTENDS Integers

CONSTANTS NumThreads, NULL
Threads == 1..NumThreads
ASSUME NULL \notin Threads

VARIABLES counter, tmp, pc, lock
vars == <<counter, tmp, pc, lock>>

Init ==
  /\ counter = 0
  /\ tmp = [t \in Threads |-> 0]
  /\ pc = [t \in Threads |-> "GetLock"]
  /\ lock = NULL

GetLock(t) ==
  /\ pc[t] = "GetLock"
  /\ lock = NULL
  /\ lock' = t
  /\ pc' = [pc EXCEPT ![t] = "GetCounter"]
  /\ UNCHANGED <<counter, tmp>>

GetCounter(t) ==
  /\ pc[t] = "GetCounter"
  /\ tmp' = [tmp EXCEPT ![t] = counter]
  /\ pc' = [pc EXCEPT ![t] = "IncCounter"]
  /\ UNCHANGED <<counter, lock>>

IncCounter(t) ==
  /\ pc[t] = "IncCounter"
  /\ counter' = tmp[t] + 1
  /\ pc' = [pc EXCEPT ![t] = "ReleaseLock"]
  /\ UNCHANGED <<tmp, lock>>

ReleaseLock(t) ==
  /\ pc[t] = "ReleaseLock"
  /\ lock = t
  /\ lock' = NULL
  /\ pc' = [pc EXCEPT ![t] = "Done"]
  /\ UNCHANGED <<counter, tmp>>

Thread(t) == GetLock(t) \/ GetCounter(t) \/ IncCounter(t) \/ ReleaseLock(t)

AllDone == \A t \in Threads : pc[t] = "Done"

Terminating == AllDone /\ UNCHANGED vars

Next == (\E t \in Threads : Thread(t)) \/ Terminating

Spec == Init /\ [][Next]_vars

\* the counter only ever goes up
Monotonic == [][counter' >= counter]_counter

\* a held lock is released, never handed to another thread
NoStealing == [][lock # NULL => lock' = NULL]_lock

\* a thread's program counter only moves forward
Forward == [][\A t \in Threads : pc'[t] = pc[t] \/ pc[t] # "Done"]_pc
====
---- CONFIG ActionProperties ----
CONSTANTS
  NumThreads = 2
  NULL = NULL
SPECIFICATION Spec
PROPERTY Monotonic
PROPERTY NoStealing
PROPERTY Forward
====
