------------------------- MODULE RiverCrossingSimple -------------------------
(***************************************************************************)
(* The same puzzle as RiverCrossing.tla, written more plainly: one         *)
(* variable per character, and one action per kind of boat trip.           *)
(* Everyone starts on the "near" bank and must reach the "far" bank.       *)
(*                                                                         *)
(* Run: tla RiverCrossingSimple.tla                                        *)
(***************************************************************************)
VARIABLES farmer, wolf, goat, cabbage

\* The bank on the other side of the river.
Across(bank) == IF bank = "near" THEN "far" ELSE "near"

Init == farmer = "near" /\ wolf = "near" /\ goat = "near" /\ cabbage = "near"

\* The farmer rows across alone.
FarmerAlone ==
  /\ farmer' = Across(farmer)
  /\ UNCHANGED <<wolf, goat, cabbage>>

\* The farmer takes the wolf, which must be on the farmer's bank.
TakeWolf ==
  /\ wolf = farmer
  /\ farmer' = Across(farmer)
  /\ wolf' = Across(wolf)
  /\ UNCHANGED <<goat, cabbage>>

TakeGoat ==
  /\ goat = farmer
  /\ farmer' = Across(farmer)
  /\ goat' = Across(goat)
  /\ UNCHANGED <<wolf, cabbage>>

TakeCabbage ==
  /\ cabbage = farmer
  /\ farmer' = Across(farmer)
  /\ cabbage' = Across(cabbage)
  /\ UNCHANGED <<wolf, goat>>

\* Nothing is eaten: without the farmer, the goat is alone with neither
\* the wolf nor the cabbage.
WolfEatsGoat == wolf = goat /\ goat # farmer
GoatEatsCabbage == goat = cabbage /\ goat # farmer
Safe == ~WolfEatsGoat /\ ~GoatEatsCabbage

\* Take one trip, and only if it is safe afterwards.
Next ==
  /\ (FarmerAlone \/ TakeWolf \/ TakeGoat \/ TakeCabbage)
  /\ Safe'

Solved == farmer = "far" /\ wolf = "far" /\ goat = "far" /\ cabbage = "far"

\* Claim the puzzle cannot be solved; the counterexample is the solution.
NotSolved == ~Solved
=============================================================================
------------------------- CONFIG RiverCrossingSimple -------------------------
INIT Init
NEXT Next
INVARIANT NotSolved
=============================================================================
