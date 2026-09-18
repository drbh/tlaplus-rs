---- MODULE SubsetAssignment ----
VARIABLES x, y
Init == /\ x \subseteq {1, 2}
        /\ y = {1, 2, 3}
Next == /\ UNCHANGED y
        /\ x' \subseteq y'
Inv == /\ x \subseteq y
       /\ ENABLED (x' \subseteq {1})
====
