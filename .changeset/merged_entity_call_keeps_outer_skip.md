---
hive-router: patch
node-addon: patch
---

# Merged entity calls keep the outer `@skip`/`@include`

When two entity calls to the same subgraph merged, the merged call only kept the `@skip`/`@include` conditions both had. In `first { price { price } } first @skip(if: $skip) { price @include(if: $include) { a: price } }` the `$skip` was dropped, so with `$skip: true` the subgraph was still asked for `a`. Each side's own conditions now stay on its fields.

Two calls for the same objects under different outer conditions only share a request when the one that runs less often reads nothing the other doesn't. Otherwise a field fetched only under the outer condition, like one a `@requires` needs, would go out as `null` when that condition is false.
