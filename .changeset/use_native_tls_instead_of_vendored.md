---
executor: patch
router: patch
---

# Use native TLS instead of vendored

In this release, we've changed the TLS settings to use `native` TLS certificates provided by the OS, instead of using certificates that are bundled (`vendored`) into the router binary. 

This change provides more flexibiliy to `router` users, as you can extend and have full control over the certificates used to make subgraph requests, by extending or changing the certificates installed on your machine, or Docker container.

The `router` is using [AWS-LC](https://aws.amazon.com/security/opensource/cryptography/) as the certificate library.

## If you are using `hive-router` Crate

If you're using the `hive-router` crate as a library, the router provides the `init_rustls_crypto_provider()` function that automatically configures AWS-LC as the default cryptographic provider. You can call this function early in your application startup before initializing the router. Alternatively, you can configure your own `rustls` provider before calling router initialization. See the [`rustls` documentation](https://github.com/rustls/rustls#cryptography-providers) for instructions on setting up a custom provider.
