---- MODULE Liveness ----
(* lesson 12: temporal properties, and why they fail.

   every property so far was a safety property: nothing bad happens. a
   liveness property says something good eventually happens.

     []P        P holds in every state (same as INVARIANT P)
     <>P        P holds in some state
     <>[]P      P eventually holds and then stays true
     []<>P      P holds infinitely often
     P ~> Q     whenever P holds, Q holds then or later

   this spec is the lock from lesson 9 with the obvious liveness property:
   the counter eventually reaches NumThreads and stays there. it fails,
   because Spec == Init /\ [][Next]_vars allows a behavior that just stops.
   stuttering steps are always allowed, so without a fairness assumption
   nothing is ever required to happen. the counterexample is a "lasso": a
   path, then a state that repeats forever.

   note: <>(counter = NumThreads) would not even catch the race from lesson
   8, since it only needs the value to show up once. <>[] does catch it

   run:    tla examples/12-liveness/Liveness.tla
   expect: Temporal property Finishes violated
           State 1: Initial predicate
           Stuttering: state 1 repeats forever.                             *)
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

Finishes == <>[](counter = NumThreads)
====
---- CONFIG Liveness ----
CONSTANTS
  NumThreads = 2
  NULL = NULL
SPECIFICATION Spec
PROPERTY Finishes
====
