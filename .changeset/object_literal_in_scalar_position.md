---
graphql-tools: patch
---

# Object literals are checked outside input object positions

`ValuesOfCorrectType` only checked an object literal where an input object was expected, so `{ field(arg: { a: 1 }) }` passed validation even when `arg` is a `String`. An object literal anywhere else is now checked like any other value. Custom scalars still accept them.
