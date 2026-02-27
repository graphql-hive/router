---
node-addon: minor
hive-router-query-planner: minor
hive-router-plan-executor: patch
hive-router: patch
---

## Rename internal query-plan path segment from `Cast(String)` to `TypeCondition(Vec<String>)`

Query Plan shape changed from `Cast(String)` to `TypeCondition(Vec<String>)`.
The `TypeCondition` name better reflects GraphQL semantics (`... on Type`) and avoids string encoding/decoding like `"A|B"` in planner/executor code.

**What changed**
- Query planner path model now uses `TypeCondition` terminology instead of `Cast`.
- Type conditions are represented as a list of type names, not a pipe-delimited string.
- Node addon query-plan typings were updated accordingly:
  - `FetchNodePathSegment.TypenameEquals` now uses `string[]`
  - `FlattenNodePathSegment` now uses `TypeCondition: string[]` (instead of `Cast: string`)
