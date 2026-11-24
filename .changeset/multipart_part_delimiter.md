---
hive-router: patch
---

# Multipart subscription events reach clients without waiting for the next one

The router wrote the boundary before each part of a `multipart/mixed` subscription response and nothing after it, so a part only ended when the next one began. Clients that hand a part on once the boundary closing it arrives, such as Apollo Kotlin, got each event only when the next event or the 10-second heartbeat came. Each part is now sent together with the boundary that closes it. The response body is the same byte for byte; only the moment each boundary is sent changed.
