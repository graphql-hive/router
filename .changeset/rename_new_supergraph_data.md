---
hive-router-plan-executor: major
---

# Rename `on_supergraph_load` end hook payload `new_supergraph_data` field to `new_supergraph`

Bringing consistency across the new supergraph snapshotting practice.

```diff
fn on_supergraph_reload<'a>(
    &'a self,
    payload: OnSupergraphLoadStartHookPayload,
) -> OnSupergraphLoadStartHookResult<'a> {
    payload.on_end(|payload| {
-       let supergraph = payload.new_supergraph_data;
+       let supergraph = payload.new_supergraph;
        println!("{}", supergraph.public_schema.sdl);
        payload.proceed()
    })
}
```
