---
hive-router-config: minor
hive-router: minor
---

# Add `cors.preflight_response_headers` to attach headers to CORS preflight (OPTIONS) responses

Adds a new optional `preflight_response_headers` map to the `cors` configuration block, and to each entry under `cors.policies`. The map allows attaching arbitrary headers (e.g. `Cache-Control`, `Server-Timing`, custom `X-*` headers) to CORS preflight (OPTIONS) responses.

This is useful because the `headers` configuration block does not affect preflight responses (they are returned early by the CORS layer), so there was previously no way to control headers like `Cache-Control` for `OPTIONS` requests.

Example:

```yaml
cors:
  enabled: true
  allow_any_origin: true
  max_age: 86400
  preflight_response_headers:
    Cache-Control: "public, max-age=86400"
```
