---- MODULE Lock ----
(* lesson 9: a lock.

   the race in lesson 8 is fixed with a lock. GetLock has a guard,
   lock = NULL, and a step whose guard is false cannot happen: the thread
   waits. NULL is a model value from the config, a name that equals only
   itself. using one instead of -1 or "none" means a mixup with a real thread
   id is a type error rather than a silent bug.

   run:    tla examples/09-lock/Lock.tla
   expect: Safety checks passed

   try deleting the lock = NULL guard to get the race back                 *)
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

TypeOK ==
  /\ counter \in 0..NumThreads
  /\ tmp \in [Threads -> 0..NumThreads]
  /\ lock \in Threads \union {NULL}

Correct == AllDone => counter = NumThreads

\* a thread's copy of the counter is never ahead of the counter itself
CounterNotLtTmp == \A t \in Threads : counter >= tmp[t]
====
---- CONFIG Lock ----
CONSTANTS
  NumThreads = 2
  NULL = NULL
INIT Init
NEXT Next
INVARIANT TypeOK
INVARIANT Correct
INVARIANT CounterNotLtTmp
====
