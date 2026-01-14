---
hive-console-sdk: patch
router: patch
---

Downgrade `reqwest` to `v0.12` to avoid runtime crash from `rustls` `CryptoProvider` introduced in reqwest `v0.13`.
