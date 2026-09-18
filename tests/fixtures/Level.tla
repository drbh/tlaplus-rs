---- MODULE Level ----
EXTENDS Naturals, TLC
VARIABLES x, phase
Init == x = 0 /\ phase = TLCGet("level")
Next == /\ x' = IF x < 3 THEN x + 1 ELSE x
        /\ phase' = IF x < 3 THEN TLCGet("level") ELSE phase
Inv == phase = x /\ TLCGet("level") = x + 1
Bad == TLCGet("level") < 4
====
