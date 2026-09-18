---- MODULE EnabledOuter ----
VARIABLES u, v
L == INSTANCE EnabledLeaf WITH a <- u, b <- v
Possible == ENABLED L!Action
====
