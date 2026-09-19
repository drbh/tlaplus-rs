---------------------------- MODULE DieHard ----------------------------
\* Measure four gallons with unmarked three- and five-gallon jugs.
\* Fill, empty, or pour until the source is empty or destination full.
\* NotSolved deliberately fails: its shortest counterexample takes six moves.
EXTENDS Integers
VARIABLE water
Jugs == {"Small", "Large"}
Capacity(j) == IF j = "Small" THEN 3 ELSE 5
Init == water = [j \in Jugs |-> 0]
TypeOK ==
  /\ water \in [Jugs -> Int]
  /\ \A j \in Jugs: water[j] >= 0 /\ water[j] <= Capacity(j)
Poured(from, to) ==
  IF water[from] < Capacity(to) - water[to]
  THEN water[from] ELSE Capacity(to) - water[to]
Fill(j) ==
  /\ water[j] < Capacity(j)
  /\ water' = [water EXCEPT ![j] = Capacity(j)]
Empty(j) ==
  /\ water[j] > 0
  /\ water' = [water EXCEPT ![j] = 0]
Pour(from, to) ==
  /\ from # to /\ water[from] > 0 /\ water[to] < Capacity(to)
  /\ water' = [water EXCEPT ![from] = water[from] - Poured(from, to),
                           ![to] = water[to] + Poured(from, to)]
Next ==
  \/ \E j \in Jugs: Fill(j) \/ Empty(j)
  \/ \E from, to \in Jugs: Pour(from, to)
NotSolved == water["Large"] # 4
=======================================================================
---------------------------- CONFIG DieHard ----------------------------
INIT Init
NEXT Next
INVARIANT TypeOK
INVARIANT NotSolved
=======================================================================
