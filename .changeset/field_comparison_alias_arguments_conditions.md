---
hive-router: patch
node-addon: patch
---

# Fix fields with different arguments, aliases or conditions counted as the same

To decide whether a request already has a field, for example a key or a field a `@requires` needs, the router compared only field names. So it treated these as the same:

| Request has               | Counted as also having, before | After |
| ------------------------- | ------------------------------ | ----- |
| `price(currency: "GBP")`  | `price(currency: "EUR")`       | no    |
| `price(currency: "GBP")`  | `gbp: price(currency: "GBP")`  | no    |
| `a @include(if: $x)`      | `a @include(if: $y)`, or `a`   | no    |
| `... on Cat { whiskers }` | `... on Dog { whiskers }`      | no    |

A request that was still needed could then be dropped as a duplicate, or another request could count on a field it didn't have.

The comparison now fully checks the alias, the arguments, the conditions and the fragments.
