---- MODULE RuntimeReads ----
EXTENDS Integers, TLC
VARIABLE x
Init == x = 0
Next == UNCHANGED x
Missing == TLCGet("nosuch") = x
MissingIndex == TLCGet(0) = x
MissingName == TLCGet("s:missing") = x
====
