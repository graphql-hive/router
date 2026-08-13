---
hive-router-plan-executor: major
hive-router: patch
---

# Allow `async fn` in the `on_http_request` plugin hook

`RouterPlugin::on_http_request` now returns `OnHttpRequestHookFuture<'req>` instead of `OnHttpRequestHookResult<'req>` directly, so plugins can `await` inside - for example calling an upstream auth or feature-flag service - before the rest of the request pipeline runs.

The hook still runs on the same worker thread it started on and is never moved across threads, so the returned future does not need to be `Send`.

Update any plugin that overrides `on_http_request` to return a boxed future:

```rust
// Before
fn on_http_request<'req>(
    &self,
    payload: OnHttpRequestHookPayload<'req>,
) -> OnHttpRequestHookResult<'req> {
    self.record_request(&payload);
    payload.proceed()
}

// After
fn on_http_request<'req>(
    &'req self,
    payload: OnHttpRequestHookPayload<'req>,
) -> OnHttpRequestHookFuture<'req> {
    Box::pin(async move {
        self.record_request(&payload);
        payload.proceed()
    })
}
```

If the body reads `self` inside the `async move` block, `&self` needs to become `&'req self` - the future now holds that borrow for its own lifetime `'req`, so the borrow has to be able to live at least that long.

Plugins that don't override `on_http_request` are unaffected.

See `plugin_examples/async_http_fetch` for an example that awaits an upstream HTTP call before proceeding.
