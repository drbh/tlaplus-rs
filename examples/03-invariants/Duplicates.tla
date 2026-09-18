---- MODULE Duplicates ----
(* lesson 3: invariants.

   an algorithm that walks a sequence and reports whether it has duplicates.
   pc is the "program counter": which step runs next. the checker tries every
   sequence in S \X S \X S as a starting state, so this checks the algorithm
   on all 64 inputs at once.

   TypeInvariant bounds every variable. IsCorrect compares the algorithm's
   answer with a direct definition of uniqueness, but only once it is Done:
   => makes an invariant conditional.

   note: the first draft of IsUnique was \A i, j : s[i] # s[j], which fails
   on every sequence since i and j may be the same index

   run:    tla examples/03-invariants/Duplicates.tla
   expect: Safety checks passed                                              *)
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
       THEN /\ IF seq[index] \in seen
                 THEN is_unique' = FALSE
                 ELSE UNCHANGED is_unique
            /\ seen' = seen \union {seq[index]}
            /\ index' = index + 1
            /\ UNCHANGED pc
       ELSE /\ pc' = "Done"
            /\ UNCHANGED <<index, seen, is_unique>>
  /\ UNCHANGED seq

\* once finished the algorithm stays put, otherwise the checker reports a
\* deadlock (a state with no successor)
Done == pc = "Done" /\ UNCHANGED vars

Next == Iterate \/ Done

TypeInvariant ==
  /\ is_unique \in BOOLEAN
  /\ seen \subseteq S
  /\ index \in 1..(Len(seq) + 1)
  /\ pc \in {"Iterate", "Done"}

IsUnique(s) == \A i, j \in 1..Len(s) : i # j => s[i] # s[j]

IsCorrect == pc = "Done" => is_unique = IsUnique(seq)
====
---- CONFIG Duplicates ----
INIT Init
NEXT Next
INVARIANT TypeInvariant
INVARIANT IsCorrect
====
