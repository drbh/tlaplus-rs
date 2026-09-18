---- MODULE ActionFunctions ----
EXTENDS Naturals
VARIABLE x
Initial[n \in {0, 1}] == x = n
Change[n \in {0, 1}] == x' = n
Hold[n \in {0, 1}] == UNCHANGED x
Unbounded[n \in Nat] == x' = n
Selected == IF x = 0 THEN Change ELSE Hold
Guard == [[n \in Nat |-> TRUE] EXCEPT ![0] = FALSE]
Init == x \in {0, 1}
InvalidInit == Initial[0]
Next == Guard[1] /\ ((\E n \in {0, 1} : Selected[n]) \/ Unbounded[x])
OutsideDomain == Change[2]
Inv == x \in {0, 1}
====
