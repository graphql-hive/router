---
router: minor
---

# JWT claims caching for improved performance

**Performance improvement:** JWT token claims are now cached for up to 5 seconds, reducing the overhead of repeated decoding and verification operations. This optimization increases throughput by approximately 75% in typical workloads.

**What's changed:**
- Decoded JWT payloads are cached with a 5-second time-to-live (TTL), which respects token expiration times
- The cache automatically invalidates based on the token's `exp` claim, ensuring security is maintained

**How it affects you:**
If you're running Hive Router, you'll see significant performance improvements out of the box with no configuration needed. The 5-second cache provides an optimal balance between performance gains and cache freshness without requiring manual tuning.
