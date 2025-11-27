---
node-addon: patch
router: patch
query-planner: patch
---

Fixed an issue whre `@skip` and `@include` directives were incorrectly removed from the initial Fetch of the Query Plan.
