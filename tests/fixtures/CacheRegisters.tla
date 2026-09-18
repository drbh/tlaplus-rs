---- MODULE CacheRegisters ----
EXTENDS Integers, TLC, TLCExt
VARIABLES x, y
Count == 1
ASSUME TLCSet(Count, 0)
Inc(v) == CASE TLCSet(Count, TLCGet(Count) + 1) -> v
First == TLCCache(Inc(1..5), {1})
Second == TLCCache(Inc({i \in 1..5 : i + 1 \in First}), {2})
ASSUME 5 \in First
ASSUME 5 \notin Second
Init == y = First /\ x = Second
Next == /\ 1 \in First
        /\ 1 \in Second
        /\ Assert(TLCGet(Count) = 2, <<"count was", TLCGet(Count)>>)
        /\ UNCHANGED <<x, y>>
====
