---
graphql-tools: patch
hive-router: patch
---

# Validate the range of `Int` literals

`ValuesOfCorrectType` validation rule now rejects `Int` literals outside the 32-bit range, for example `query ($v: Int = 2147483648)`, with `Int cannot represent non 32-bit signed integer value: 2147483648`.
