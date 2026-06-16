---
hive-router-config: minor
hive-router-plan-executor: minor
---

# Support VRL expression for subscription callback `public_url`

The `subscriptions.callback.public_url` config field now accepts either a static URL string or a VRL expression, in addition to the previously supported plain URL value.

This is useful in horizontally scaled deployments where the public callback URL is not known at build time and must be resolved at runtime - for example, from an environment variable set by the orchestrator per instance.

## Configuration

```yaml
subscriptions:
  enabled: true
  callback:
    # static URL (existing behavior, unchanged)
    public_url: "https://my-router.example.com/callback"
    subgraphs:
      - reviews
```

```yaml
subscriptions:
  enabled: true
  callback:
    # VRL expression - resolved at runtime
    public_url:
      expression: 'env("ROUTER_CALLBACK_PUBLIC_URL")'
    subgraphs:
      - reviews
```
