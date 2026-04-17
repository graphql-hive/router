---
hive-router: patch
---

Adds an optional `graphiql` Cargo feature for `hive-router`.
When enabled, the Router serves GraphiQL HTML and skips Laboratory asset generation so `npm` and `node` dependencies are not needed.
By default, this feature is disabled and existing Laboratory behavior is unchanged.

```bash
cargo run -p hive-router --features graphiql
cargo build -p hive-router --features graphiql
```
