---
router: patch
executor: patch
---

Fixed response projection for fields on different concrete types of interfaces and unions.

When a query included fragments on an abstract type (interface or union) that selected fields with the same name but different return types, the projection would incorrectly use a single, merged plan for all types. This caused projection to fail when processing responses where different concrete types had different field implementations.

For example, with `... on A { children { id } }` and `... on B { children { id } }` where `A.children` returns `[AChild]` and `B.children` returns `[BChild]`, the projection would fail to correctly distinguish between the types and return incomplete or incorrect data.

The fix introduces type-aware plan merging, which preserves the context of which concrete types a field came from. During response projection, the type is now determined dynamically for each object, ensuring the correct field type is used.
