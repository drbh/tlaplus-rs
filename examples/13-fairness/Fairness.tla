---- MODULE Fairness ----
(* lesson 13: fairness.

   the fix for lesson 12 is to say the threads keep running. WF_vars(A) is
   weak fairness: if A stays enabled forever, it eventually happens. SF_vars(A)
   is strong fairness: if A is enabled infinitely often, even on and off, it
   eventually happens. fairness goes into Spec with /\ and the checker then
   ignores behaviors that break it.

   here each thread is weakly fair on its own. once thread 1 is Done, thread
   2's GetLock stays enabled, so weak fairness is enough. a thread that could
   be starved by others grabbing the lock first would need SF.

   the properties: every thread that wants the lock eventually finishes
   (~>), and the counter ends up correct and stays there (<>[]).

   run:    tla examples/13-fairness/Fairness.tla
   expect: Temporal property Finishes passed
           Temporal property EveryThreadFinishes passed
           Safety checks passed                                              *)
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

Spec ==
  /\ Init
  /\ [][Next]_vars
  /\ \A t \in Threads : WF_vars(Thread(t))

Finishes == <>[](counter = NumThreads)

EveryThreadFinishes == \A t \in Threads : pc[t] = "GetLock" ~> pc[t] = "Done"
====
---- CONFIG Fairness ----
CONSTANTS
  NumThreads = 3
  NULL = NULL
SPECIFICATION Spec
PROPERTY Finishes
PROPERTY EveryThreadFinishes
====
