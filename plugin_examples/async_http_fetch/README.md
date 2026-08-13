# Async HTTP Fetch Plugin Example

Demonstrates calling out to an upstream HTTP service from `on_http_request` and awaiting the
response before the rest of the request pipeline (parsing, validation, planning, execution) runs.

The fetched value is stored in the per-request `PluginContext` and surfaced back on the response
as the `x-fetched-greeting` header via `on_end`.

```yaml
plugins:
  async_http_fetch:
    enabled: true
    config:
      upstream_url: http://0.0.0.0:9876/greeting
```

The upstream is expected to respond with:

```json
{ "greeting": "hello from upstream" }
```
