---
hive-router: patch
---

# Drop messages instead of completing subscriptions in the HTTP callback transport

When an HTTP callback subscription's internal buffer is full, acknowledge the callback and drop only that message instead of returning a 503 response and terminating the subscription. This aligns HTTP callback behavior with the other streaming transports and lets slow consumers recover without reconnecting.
