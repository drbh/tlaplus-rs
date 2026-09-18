---- MODULE DuplicatesBug ----
(* lesson 4: reading a counterexample.

   the same algorithm as lesson 3 with a bug: is_unique is set to FALSE on
   every element, seen or not. the spec runs fine, it is IsCorrect that
   catches it.

   the trace is the shortest path from an initial state to the violation.
   each state is labeled with the action that produced it, and * marks the
   variables that changed. the checker exits 1 on a violation.

   run:    tla examples/04-counterexample/DuplicatesBug.tla
   expect: Invariant IsCorrect is violated.
           State 1: Initial predicate  (a sequence with no duplicates)
           ... State 5: Iterate        (pc = "Done", is_unique = FALSE)      *)
EXTENDS Integers, Sequences, FiniteSets

S == 1..4

VARIABLES seq, index, seen, is_unique, pc
vars == <<seq, index, seen, is_unique, pc>>

Init ==
  /\ seq \in S \X S \X S
  /\ index = 1
  /\ seen = {}
  /\ is_unique = TRUE
  /\ pc = "Iterate"

Iterate ==
  /\ pc = "Iterate"
  /\ IF index <= Len(seq)
       THEN /\ is_unique' = FALSE                  \* the bug
            /\ seen' = seen \union {seq[index]}
            /\ index' = index + 1
            /\ UNCHANGED pc
       ELSE /\ pc' = "Done"
            /\ UNCHANGED <<index, seen, is_unique>>
  /\ UNCHANGED seq

Done == pc = "Done" /\ UNCHANGED vars

Next == Iterate \/ Done

IsUnique(s) == \A i, j \in 1..Len(s) : i # j => s[i] # s[j]

IsCorrect == pc = "Done" => is_unique = IsUnique(seq)
====
---- CONFIG DuplicatesBug ----
INIT Init
NEXT Next
INVARIANT IsCorrect
====
