---
hive-console-sdk: minor
hive-router: patch
---

# Negative Cache and Single-Flight

Introduced single-flight resolution of documents in the SDK.

Added a negative cache to store non 2XX requests for 5s (configurable, but in SDK it's disabled by default). It's meant to not keep repeating the same requests that eventually give errors or 404s.
