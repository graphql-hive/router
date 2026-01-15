---
executor: patch
router: patch
---

# Use native TLS instead of vendored

In this release, we've changed the TLS settings to use `native` TLS certificates provided by the OS, instead of using certificates that are bundled (`vendored`) into the router binary. 

This change provides more flexibiliy to `router` users, as you can extend and have full control over the certificates used to make subgraph requests, by extending or changing the certificates installed on your machine, or Docker container.

The `router` is using [AWS-LC](https://aws.amazon.com/security/opensource/cryptography/) as the certificate library.

## If you are using `hive-router` Crate

Users who depends on `hive-router` crate and use it as a library, will need to configure the `rustls` provider that they prefer. See [`rustls` README](https://github.com/rustls/rustls#cryptography-providers) for instructions.
