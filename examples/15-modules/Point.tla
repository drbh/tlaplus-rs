---- MODULE Point ----
EXTENDS Integers

CONSTANTS X, Y
ASSUME X \in Int /\ Y \in Int

Repr == <<X, Y>>

Add(x, y) == <<X + x, Y + y>>
====
