---
graphql-tools: patch
router: patch
executor: patch
hive-console-sdk: patch
---

# Add `minify_query_document` for optimized query minification

Implements `minify_query_document` to minify parsed GraphQL operations directly, avoiding the need for an intermediate `Display` step. This new approach uses `itoa` and `ryu` for efficient integer and float formatting.

By minifying the query document representation instead of the query string, we achieve performance improvements: query minification time is reduced from 4μs to 500ns, and unnecessary allocations are eliminated.

Includes benchmarks and tests to validate the performance gains and correctness of the new implementation.
