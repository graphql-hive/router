---
hive-router: patch
node-addon: patch
---

# Keep `@provides` fields on the path that provides them

Fields made available by `@provides` could leak to another field returning the same type, like `Store.orders` getting the fields `User.orders` provides. The planner then skipped a fetch it needed. Provided fields now only count on the path of the field that provides them.
