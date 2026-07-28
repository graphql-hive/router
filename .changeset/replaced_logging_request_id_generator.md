---
hive-router-internal: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Replaced logging request-id generator

Version `0.0.85` introduced new logging runtime that uses `Sonyflake` crate to generate request-id in requests that doesn't have it passed via HTTP headers. 

The `Sonyflake` runtime can fail on some OS configurations. 

This change replaces the `Sonyflake` runtime with `Ulid` generator, that's more fail-safe.
