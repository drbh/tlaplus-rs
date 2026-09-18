---- MODULE Deadlock ----
(* lesson 10: deadlock.

   two threads and two locks. thread 1 takes a then b, thread 2 takes b then
   a. if each grabs its first lock before the other grabs its second, both
   wait forever.

   a deadlock is a state with no enabled step. the checker reports it by
   default, which is also why the earlier specs needed a Terminating step:
   finishing is not deadlock, only the trace tells them apart. pass
   --no-deadlock (or CHECK_DEADLOCK FALSE in the config) when a stopped
   system is fine.

   run:    tla examples/10-deadlock/Deadlock.tla
   expect: Deadlock reached.
           a trace ending with holder = [a |-> 1, b |-> 2]                   *)
EXTENDS Integers

CONSTANT NULL
Threads == {1, 2}
Locks == {"a", "b"}

\* the order each thread acquires the locks in
Order == (1 :> <<"a", "b">>) @@ (2 :> <<"b", "a">>)

VARIABLES holder, held
vars == <<holder, held>>

Init ==
  /\ holder = [l \in Locks |-> NULL]
  /\ held = [t \in Threads |-> 0]

Acquire(t) ==
  /\ held[t] < 2
  /\ LET l == Order[t][held[t] + 1]
     IN /\ holder[l] = NULL
        /\ holder' = [holder EXCEPT ![l] = t]
  /\ held' = [held EXCEPT ![t] = @ + 1]

Release(t) ==
  /\ held[t] = 2
  /\ holder' = [l \in Locks |-> IF holder[l] = t THEN NULL ELSE holder[l]]
  /\ held' = [held EXCEPT ![t] = 0]

Next == \E t \in Threads : Acquire(t) \/ Release(t)

TypeOK ==
  /\ holder \in [Locks -> Threads \union {NULL}]
  /\ held \in [Threads -> 0..2]
====
---- CONFIG Deadlock ----
CONSTANT NULL = NULL
INIT Init
NEXT Next
INVARIANT TypeOK
====
