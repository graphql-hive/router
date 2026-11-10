---
hive-router: patch
---

# Entity calls keep the conditions they were planned under

An entity call under nested `@include`/`@skip`, like `orders @include(if: $a) { ... on Order @include(if: $b) { name } }`, only kept the innermost condition and relied on the call above it for the rest. When another branch fetched the same objects, or the call merged with an unconditional sibling, it ran with the outer condition off. Every call now keeps all the conditions it was planned under.
