---
hive-router: patch
---

# Improve restrictive `Cache-Control` merging

The router now preserves and safely merges standard `Cache-Control` directives instead of discarding an entire subgraph value when it contains a directive that was not previously modeled. This prevents restrictions such as `no-store` or `private` from being lost when they appear alongside another standard directive.

The merge now supports `s-maxage`, `stale-while-revalidate`, and `stale-if-error` by selecting the minimum applicable duration. `s-maxage` is merged with awareness that shared caches prefer it over `max-age`, so mixed directives cannot extend the effective shared-cache lifetime.

`private` now overrides `public` without being escalated to `no-store, no-cache`. Other directives are retained, allowing results such as `private, max-age=50` to remain available to private caches while still preventing shared-cache storage.

The router also preserves `proxy-revalidate`, `must-understand`, and `no-transform` when any subgraph sets them. `immutable` is preserved only when every subgraph sets it, like `public`. Qualified `no-cache="field"` and `private="field"` forms are treated as their more restrictive unqualified forms.

Unknown directives, missing or non-numeric duration values, and other malformed directives now conservatively produce `no-store, no-cache` instead of allowing the affected subgraph value to be ignored.
