---
name: test-audit
description: "Review, write, or remove Hive Router tests: reject duplicate and implementation-coupled tests while retaining independent GraphQL, federation, security, and runtime contracts."
---

# Hive Router Test Audit

Use this skill whenever writing, changing, reviewing, or removing tests in Hive Router. The goal is **regression detection per maintenance cost**, not test count, coverage percentage, or deletion count.

## Gate every new or changed test

Before writing the test, answer:

1. **Contract:** Which externally observable behavior, invariant, or independently specified contract does it protect?
2. **Regression:** What credible bug would make the test fail, and would it fail for the right reason?
3. **Ownership:** Which existing test already covers that behavior? Why does this test need to exist at another boundary?
4. **Seams:** Does it require a production export, flag, wrapper, injection point, or special branch only for testing? Prefer proof at a real boundary.
5. **Cost:** Is the setup, runtime, brittleness, and ongoing maintenance justified by the unique risk caught?

If these cannot be answered, do not add the test. Prefer improving one existing test, a table-driven case, or a shared fixture over adding another owner.

For regressions, demonstrate a red test against the unfixed behavior when practical, then green after the fix. If baseline reproduction is unavailable, state that explicitly; never claim red/green validation without it.

## Choose the strongest meaningful boundary

- **Query planner:** Test the externally meaningful plan semantics: correct fetch dependencies, conditions, variable use, entity paths, service selection, and valid results. For a tricky planning bug, a focused planner fixture is useful; avoid snapshotting huge incidental plan trees or asserting internal search steps when an equivalent semantic assertion works. Retain exact plan snapshots only when their shape or ordering is deliberately a compatibility or performance-sensitive contract.
- **Plan executor and response pipeline:** Test final GraphQL data/errors, null propagation, aliases, abstract types, conditional selections, entity merging, deduplication, incremental delivery and streaming behavior as applicable. Do not test private slot indices, arena offsets, container choices, allocation layout, or internal function calls unless a documented invariant truly depends on them.
- **End-to-end router:** Prefer this boundary for observable HTTP behavior, subgraph interaction, auth/security, headers, transport negotiation, retries, errors, streaming, and deployment/config lifecycle. Do not repeat the same logical regression in unit, executor and E2E suites unless each layer protects a distinct failure mode.
- **Parsing and JSON handling:** Cover valid/invalid inputs, escapes, number boundaries, unknown fields, malformed responses, and GraphQL-relevant semantics. For fuzzers and differential tests, use an independent oracle or well-defined invariant; do not derive expected values from the implementation under test.
- **Configuration and integrations:** Retain tests for defaults, YAML/env precedence, validation, plugin API contracts, OTel, persisted documents, and compatibility boundaries. Mock external services when necessary but never let the mock implement the exact behavior being asserted.
- **Performance:** Use benchmarks for latency, throughput, allocation and memory regressions. Functional tests should protect semantics; do not hardcode unstable timing thresholds into ordinary unit tests. Preserve dedicated performance contracts when measured in a stable and documented environment.

## Reject low-value tests

Remove or rewrite tests that:
- Compare a result with itself, or construct expected output from the function under test.
- Assert only source text, names, private call counts, hash-map versus array choice, or implementation-specific ordering.
- Replay the same contract across layers without a separate transport or lifecycle risk.
- Copy every enum, field, plan node, or fixture inventory without protecting an independently specified public contract.
- Depend on a mock that already supplies the answer they claim to verify.
- Pass when the tested implementation is removed, replaced with a constant, or deliberately broken.
- Require test-only production seams with no actual runtime caller.
- Break during a behavior-preserving refactor.

Don't delete a test merely because it is slow, snapshot-based, or lower-level. First identify the independent contract it protects.

## Never discard high-value proof

Protect GraphQL specification compliance, federation interoperability and audit cases, planner correctness, null/error behavior, authorization and scope enforcement, input validation, HTTP and streaming protocols, schema reloads, config defaults, persistence, plugin integrations, and regressions reported by real users.

Keep distinct tests for the same feature when they expose genuinely different failure modes (for example, planner dependency semantics versus actual HTTP subgraph request scheduling).

## Audit workflow

1. Scope to one owner/module or one coherent feature; do not run a repository-wide deletion campaign by default.
2. Inspect the full test, implementation, callers, shared fixtures, nearby tests, and relevant bug or PR history.
3. Write down what independently specified behavior each candidate protects and what surviving test will still catch its regression.
4. Classify each candidate **KEEP**, **IMPROVE**, **MERGE**, or **DELETE**, with a concrete reason. If proof would disappear, keep or replace it before deleting.
5. For tests to merge, preserve meaningful edge cases while reducing duplicated setup and assertions.
6. If tests caused production-only exports, hooks or flags, remove those seams when they have no real production caller.
7. Run the narrow affected tests first, then neighboring owner and relevant integration tests. The repository documents `cargo test_all`, `cargo test_e2e`, `cargo test_qp`, and `cargo test_qpe`; use the suites relevant to the change. Never claim commands passed unless actually run.
8. Review the diff for lost behavior, unrelated deletions, altered snapshots, accidental fixture weakening, and new brittle coupling.

## Reporting

For every audit or test-oriented change, report:
- What behavioral contracts are retained or added and which redundant assertions were removed.
- Why each deletion is safe, including the remaining proof owner.
- Tests actually run, their results, and any verification gaps.
- Production versus test code changed, plus any follow-up risks.

**Principle:** Fewer tests are better only when the surviving tests catch the same important regressions with less duplication and less fragility.
