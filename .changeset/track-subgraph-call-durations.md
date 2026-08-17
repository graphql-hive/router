---
hive-router: minor
---

# Track per-subgraph call durations on the request summary

The request summary now tracks `subgraph_calls_duration`, a map of subgraph name to the durations of every call made to it during the request. 

This is available to custom plugins via `get_current_summary()`, alongside the existing subgraph tracking fields.
