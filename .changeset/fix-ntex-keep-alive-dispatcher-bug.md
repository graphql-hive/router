---
hive-router: patch
---

Upgrade `ntex` dependencies

Bumps `ntex` to 3.12.3 (and its `ntex-*` sibling crates to matching versions) to pick up an upstream fix (ntex-rs/ntex#933, released in ntex 3.10.1) for a dispatcher bug where an idle keep-alive connection's timer was shadowed whenever a request-header read timeout was also configured.
