---- MODULE ModelValues ----
EXTENDS Naturals, Sequences, TLC, TLCExt
CONSTANT Known
VARIABLES x, name
Label(n) == TLCModelValue("k_" \o ToString(n))
Init == x \in {0, 1} /\ name = Label(x)
Next == /\ x' = IF x < 2 THEN x + 2 ELSE x - 2
        /\ name' = Label(x')
Inv == /\ name = Label(x)
       /\ (x = 0 => name = Known)
       /\ (x # 0 => name # Known)
====
