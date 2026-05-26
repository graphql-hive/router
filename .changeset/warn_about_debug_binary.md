---
hive-router: patch
---

# Warn about debug binary in production use 

Added a warning message to the router binary to warn users about using a debug binary in production use.

This is intended to prevent cases where users accidentally use a debug binary in production, which can lead to performance issues.
