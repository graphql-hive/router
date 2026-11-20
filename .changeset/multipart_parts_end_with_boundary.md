---
hive-router: patch
---

# Multipart responses reach the client one part sooner

A `multipart/mixed` response sent the boundary that ends a part only in front of the next part. Clients wait for that boundary before they hand a part over, so a subscription event only showed up when the next event or the next heartbeat came. Every part now goes out with the boundary that ends it. The bytes of the response are the same, they're just sent sooner.

This applies to subscriptions over `multipart/mixed`, both the incremental delivery format and the multipart subscription protocol (`subscriptionSpec`), and to `@defer` responses.
