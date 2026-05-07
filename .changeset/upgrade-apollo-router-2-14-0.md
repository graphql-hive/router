---
apollo-router-hive-fork: patch
---

Upgrade `apollo-router` from `2.13.1` to `2.14.0` to pull in `hickory-proto 0.26.1` which fixes [GHSA-3v94-mw7p-v465](https://github.com/advisories/GHSA-3v94-mw7p-v465) — an unbounded loop in DNSSEC NSEC3 closest-encloser proof validation that can exhaust memory. No patched `0.25.x` release of `hickory-proto` exists; the fix only landed in the `0.26.x` line, which requires `apollo-router ≥ 2.14.0`.

The Rust toolchain is bumped from `1.94.1` to `1.95.0` in `apollo-router-workspace/rust-toolchain.toml` because `apollo-router 2.14.0` declares `rust-version = "1.95.0"` in its manifest.
