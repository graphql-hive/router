---
hive-router-plan-executor: patch
hive-router: patch
---

# Fix timeout error message to include the timeout duration instead of the endpoint URL

Previously by mistake, the error message for subgraph request timeouts included the endpoint URL instead of the timeout duration like `Request to subgraph timed out after http://ACCOUNT_ENDPOINT:PORT/accounts milliseconds`. This change simplifies the error message like `Request to subgraph timed out`.