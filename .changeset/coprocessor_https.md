---
hive-router: minor
---

# Support `https://` coprocessor URLs

`coprocessor.url` now accepts `https://` endpoints, and a new `coprocessor.tls` block configures how the router connects to them. It takes the same settings as `traffic_shaping.subgraphs.<name>.tls`: a custom CA (`cert_file`), a client certificate for mTLS (`client_auth`), and `insecure_skip_ca_verification`.

```yaml
coprocessor:
  url: https://coprocessor.internal:8443/hook
  protocol: http2
  tls:
    cert_file: ./certs/coprocessor-ca.pem
    client_auth:
      cert_file: ./certs/router.pem
      key_file: ./certs/router.key
```

- Without a `tls` block, the coprocessor's certificate is verified against the system's root certificates. `coprocessor.tls` does not inherit `traffic_shaping.all.tls`.
- `protocol: http1` and `protocol: http2` are supported over `https://`. `http2` was previously rejected; it now requires an `https://` endpoint, and `h2c` requires `http://` or `unix://`.
- Setting `tls` on an `http://` or `unix://` endpoint fails at startup.
- The per-stage `timeout` includes the TLS handshake when a new connection is opened.

Closes https://github.com/graphql-hive/router/issues/1731
