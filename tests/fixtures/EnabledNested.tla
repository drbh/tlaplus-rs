---- MODULE EnabledNested ----
EXTENDS Naturals
VARIABLE x
I == INSTANCE EnabledOuter WITH u <- x, v <- x
Init == x = 0
Next == x' = 1 - x
Inv == I!Possible
====
