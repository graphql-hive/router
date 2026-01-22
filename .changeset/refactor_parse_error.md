---
graphql-tools: minor
hive-console-sdk: patch
router: patch
query-planner: patch
plan-executor: patch
node-addon: patch
---

# Refactor Parse Error Handling

Breaking;
- `ParseError(String)` is now `ParseError(InternalError<'static>)`.
- - So that the internals of the error can be better structured and more informative, such as including line and column information.
- `ParseError`s are no longer prefixed with "query parse error: " in their Display implementation.
