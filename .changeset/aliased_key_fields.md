---
hive-router: patch
node-addon: patch
---

Send an entity to its subgraph when the client uses the name of one of its key fields for something else.

In `{ t { x: id  id: y } }`, the client's `id` is a different field, so the router fetches the key `id` under an internal alias. The check for a complete key looked at `id` in the response instead, found nothing there before the entity call, and left the entity out, so `y` came back `null`. With `{ t { x: id  id: x  y } }`, the answer depended on which root fetch finished first. The check now looks at the key the representation actually read.

A required field that the planner aliased is also read only from that alias now. When it was missing, the router used to send whatever sat under the field's name instead, like the client's `price(currency: USD)` as the `price(currency: EUR)` a `@requires` asked for.
