---------------------------- MODULE RiverCrossing ----------------------------
(***************************************************************************)
(* A farmer must take a wolf, a goat and a cabbage across a river.  The    *)
(* boat carries the farmer and at most one passenger.  Left alone, the     *)
(* wolf eats the goat and the goat eats the cabbage.  As with DieHard, we  *)
(* find a solution by asserting that the puzzle cannot be solved: the      *)
(* counterexample to NotSolved is the sequence of crossings.               *)
(*                                                                         *)
(* The configuration is the CONFIG section after the module; run           *)
(*   tla RiverCrossing.tla   (TLC: tlc -config RiverCrossing.tla ...)      *)
(***************************************************************************)
VARIABLE side  \* The bank, "L" or "R", each of the four is on.

Items == {"farmer", "wolf", "goat", "cabbage"}
Other(b) == IF b = "L" THEN "R" ELSE "L"

TypeOK == side \in [Items -> {"L", "R"}]

Init == side = [i \in Items |-> "L"]

\* Nothing is eaten: the goat is never left with the wolf or the cabbage.
Safe ==
  side["goat"] = side["farmer"]
    \/ (side["goat"] # side["wolf"] /\ side["goat"] # side["cabbage"])

\* The farmer crosses alone (passenger "farmer") or with an item on the same
\* bank, and may only leave the banks safe.
Cross(passenger) ==
  /\ side[passenger] = side["farmer"]
  /\ side' = [i \in Items |->
                IF i \in {"farmer", passenger} THEN Other(side[i]) ELSE side[i]]
  /\ Safe'

Next == \E p \in Items : Cross(p)

Solved == \A i \in Items : side[i] = "R"

NotSolved == ~Solved
=============================================================================
---------------------------- CONFIG RiverCrossing ----------------------------
INIT Init
NEXT Next
INVARIANT TypeOK
INVARIANT NotSolved
=============================================================================
