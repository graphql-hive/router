---
graphql-tools: patch
---

# Lists of different lengths are different values

`OverlappingFieldsCanBeMerged` treated a list argument and any prefix of it (`[]` included) as the same value, so `f(input: ["a"])` and `f(input: [])` could share a response key.
