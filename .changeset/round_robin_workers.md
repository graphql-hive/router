---
hive-router: patch
---

# Spread new connections over all HTTP workers

The HTTP server now hands new client connections to its workers in turn. Before, it sent each connection to the less busy of two random workers. When many connections opened at once, a worker that had just been busy looked busy for all of them and could get none, so it sat idle while the other workers queued requests. With 4 workers and 50 connections opened together, one worker regularly got no connections at all.
