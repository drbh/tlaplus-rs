---- MODULE RecursiveProjection ----
EXTENDS Naturals
VARIABLE x
Init == x = 2
Next == UNCHANGED x
f[i \in 0..5] == [j \in 0..5 |->
    IF j = 0 THEN [k \in 0..i |-> IF k = 0 THEN i ELSE f[i][1] + 1]
    ELSE IF i = 0 THEN j ELSE f[i-1][j] + 1]
g[i \in 0..5] == [j \in 0..i |->
    IF j = 0 THEN IF i = 0 THEN 0 ELSE g[i-1][i-1]
    ELSE g[i][j-1] + x]
h[i \in 0..3, j \in 0..3] == [k \in 0..3 |->
    IF k = 0 THEN IF j = 0 THEN IF i = 0 THEN 0 ELSE h[i-1,j][k] + 1
                           ELSE h[i,j-1][k] + 1
    ELSE h[i,j][k-1] + 1]
Inv == /\ f[4][3] = 7
       /\ f[4][0][2] = 6
       /\ DOMAIN g[3] = 0..3
       /\ DOMAIN f[5][0] = 0..5
       /\ g[0][0] = 0
       /\ g[3][3] = 12
       /\ g[3][1] = 8
       /\ g[2][0] = 2
       /\ DOMAIN h[1,2] = 0..3
       /\ DOMAIN h = (0..3) \X (0..3)
       /\ h[2,3] = [i \in 0..3 |-> i + 5]
       /\ h[1,2][3] = 6
====
