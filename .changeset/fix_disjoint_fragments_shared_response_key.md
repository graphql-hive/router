---
hive-router-plan-executor: patch
hive-router: patch
---

# Fix shared response keys leaking across disjoint fragments

When several fragments on different types selected the same response key with different sub-selections (e.g. `... on Article { meta { title } }` vs `... on Video { meta { wordCount } }`), the response projection merged their child selections together instead of keeping them per type.

As a result, a sub-field selected for one type could leak into the projection of another. 

When that leaked field was Non-Null and absent from the actual type's data, null propagation bubbled up and collapsed the whole response to `data: null`.

Fixes [#1166](https://github.com/graphql-hive/router/pull/1166)
