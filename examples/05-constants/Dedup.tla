---- MODULE Dedup ----
(* lesson 5: constants, model values and symmetry.

   lesson 3 hardcoded S == 1..4 and sequences of length 3. CONSTANT moves
   those choices into the config so the same spec checks small inputs
   quickly and large ones overnight. ASSUME rejects nonsense values early.

   the config gives S a set of model values {s1, s2, s3}: opaque names that
   only support = and #. the algorithm never does arithmetic on elements, so
   any permutation of s1 s2 s3 behaves the same. SYMMETRY tells the checker
   to explore one representative of each permutation, which is why the state
   count is far below what 1..Size over three values would produce.

   seq \in [1..n -> S] for every n in 1..Size is "state sweeping": all
   sequences up to length Size are checked in one run.

   run:    tla examples/05-constants/Dedup.tla
   expect: Safety checks passed                                              *)
EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS S, Size
ASSUME Cardinality(S) >= 1
ASSUME Size \in 1..10

Symmetry == Permutations(S)

VARIABLES seq, index, seen, is_unique, pc
vars == <<seq, index, seen, is_unique, pc>>

Init ==
  /\ \E n \in 1..Size : seq \in [1..n -> S]
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

Done == pc = "Done" /\ UNCHANGED vars

Next == Iterate \/ Done

TypeInvariant ==
  /\ is_unique \in BOOLEAN
  /\ seen \subseteq S
  /\ index \in 1..(Len(seq) + 1)

IsUnique(s) == \A i, j \in 1..Len(s) : i # j => s[i] # s[j]

IsCorrect == pc = "Done" => is_unique = IsUnique(seq)
====
---- CONFIG Dedup ----
CONSTANTS
  S = {s1, s2, s3}
  Size = 4
SYMMETRY Symmetry
INIT Init
NEXT Next
INVARIANT TypeInvariant
INVARIANT IsCorrect
====
