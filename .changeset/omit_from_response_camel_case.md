---
hive-router: patch
node-addon: patch
---

# `omitFromResponse` in query plans

A required field that is left out of the response is now marked `omitFromResponse` in the query plan's JSON, in camelCase like every other field, instead of `omit_from_response`.
