---- MODULE SequenceBounds ----
EXTENDS Integers, Sequences
VARIABLE x
Init == x = 0
Next == UNCHANGED x
TupleReversed(unused) == <<SubSeq(<<1, 2>>, 0, -1), SubSeq(<<1, 2>>, 5, 4), SubSeq(<<>>, 0, -1)>>
StringReversed(unused) == <<SubSeq("ab", 0, -1), SubSeq("ab", 5, 4), SubSeq("", 0, -1)>>
ExtremeReversed(unused) == <<SubSeq(<<>>, 2147483647, -2147483647), SubSeq("", 2147483647, -2147483647)>>
Valid(unused) == <<SubSeq(<<1, 2>>, 1, 2), SubSeq("ab", 2, 2), Len("ab"), Append(<<1>>, 2)>>
LowerBound(unused) == SubSeq(<<1, 2>>, 0, 1)
UpperBound(unused) == SubSeq(<<1, 2>>, 1, 5)
StringBound(unused) == SubSeq("ab", 1, 5)
BothBounds(unused) == SubSeq(<<1, 2>>, 3, 5)
EmptyBounds(unused) == SubSeq(<<>>, 1, 1)
SecondType(unused) == SubSeq(<<1>>, TRUE, 1)
ThirdType(unused) == SubSeq(<<1>>, 1, TRUE)
FirstType(unused) == SubSeq(1, 1, 1)
InvalidLen(unused) == Len(1)
InvalidHead(unused) == Head(1)
InvalidTail(unused) == Tail(1)
EmptyStringTail(unused) == Tail("")
InvalidAppend(unused) == Append(1, 2)
BadBound == UpperBound(x) = <<>>
BadLen == InvalidLen(x) = 0
BadAppend == InvalidAppend(x) = <<>>
BadTail == EmptyStringTail(x) = ""
====
