---
node-addon: patch
---

# Include `@context` / `@fromContext` fields in the published query plan types

`FetchNode` and `EntityBatchAlias` declare the optional `contextRewrites` and `contextVariables` fields, and `FetchNodePathSegment` includes the `"Parent"` variant.
