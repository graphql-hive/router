---
hive-router: patch
---

# Error paths in a subgraph response are no longer rebuilt at every step

When a subgraph response carries errors, the router walks the response data building a GraphQL error
path for each entity it visits. It built that path by cloning: the whole path accumulated so far was
copied, and one segment appended, once per level per visited item. The path is now carried through
the walk and pushed and popped as it descends, so only the leaf - the one place that needs to own a
path - pays for one.

A transcription of the two walks over the same data, counting allocations, puts the saving at around
a third for the shallow two-level paths that dominate, and at half to three fifths for deeper ones.
This only runs when a subgraph returns an error; responses without errors build no path in either
form.
