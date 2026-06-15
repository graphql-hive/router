---
hive-router: patch
---

# Fix: process initial GraphiQL variables

The Yoga GraphiQL wrapper reads query from the current URL, but it was not reading the variables URL parameter. 

This change now allows GraphiQL to process the `variables` from query parameters.
