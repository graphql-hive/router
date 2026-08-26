---
hive-router: minor
---

Add `jwt.forward_claims_to_upstream_extensions.include_claims` to forward only specific JWT claims to subgraphs

Previously, enabling `forward_claims_to_upstream_extensions` always forwarded the entire JWT payload under `extensions.<field_name>`. You can now restrict this to a list of root-level claim keys:

```yaml
jwt:
  forward_claims_to_upstream_extensions:
    enabled: true
    field_name: jwt
    include_claims:
      - sub
      - org_id
```

If `include_claims` is not set, all claims are forwarded as before.

Closes https://github.com/graphql-hive/router/issues/642
