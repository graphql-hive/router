---
hive-router: patch
---

# Subgraph request text is stored as a boxed string

The text of a subgraph request is built once when the plan is built, and never added to after that. It was stored in a `String`, which can hold more memory than the text needs.

The text is now stored as a boxed string, which holds exactly the bytes it needs.
