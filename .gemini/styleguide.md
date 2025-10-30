# Gemini Code Assistant – Style Guide
## Performance-First, Readability-Always (Super-Performant & Clear)

> TL;DR: Treat performance as a feature **and** clarity as a constraint. We ship **super-performant code** that is easy to read, easy to change, and hard to misuse. Prefer zero/low-overhead paths, avoid accidental allocations, and never pay logging costs on hot paths unless explicitly gated. Keep control flow shallow and intent obvious.

---

## Quick Facts (Tracing Semantics You Must Know)

- **Default `#[instrument]` level is `debug`.**
- Tracing levels are ordered: `error > warn > info > debug > trace`. Enabling `info` **does not** enable `debug`.
- **Field expressions inside `#[instrument(... fields(...))]` are evaluated at function entry** (before any subscriber decides to record or drop the span). If those expressions do heavy work (e.g., `pretty_print`, `format!`, `to_string`, serialization), you **pay that cost** regardless of whether the span is recorded.
- Therefore, any non-trivial work inside `fields(...)` is a foot-gun even when you think “it’s only at `debug`.”

---

## Super-Performance Ethos

- **Zero-cost baseline:** When disabled, observability adds ~0 runtime cost.
- **Hot-path budget ≈ near-zero:** Guard everything non-trivial, minimize allocs, avoid dynamic dispatch and stringification in loops, keep data on the stack/arenas/Bytes where possible.
- **Measure, don’t guess:** PRs that plausibly affect p95/p99 latency or memory should include micro-benchmarks or before/after numbers.
- **Prefer predictable memory:** Borrow over own, interning for high-cardinality strings, `SmallVec/ArrayVec` where sizes are bounded, arena/bump alloc for structured lifetimes.

---

## Blocking Policy (exact)

Gemini must **request changes (block the PR)** when any of the following appear:

### Tracing / Instrumentation
- **A. `#[instrument]` at `level = "info"` (or higher) on hot paths.**
  `info` is commonly enabled in prod; span creation + eager field eval will run in hot code. Use `level = "trace"` on hot paths and record richer details lazily when enabled.
- **B. `#[instrument]` at `debug` (default) on hot paths without `skip_all`, or with computed fields.**
  Even if disabled at runtime, `fields(...)` are evaluated. Require `skip_all` and **cheap fields only**.
- **C. Any heavy work is performed just to log/trace on a hot path** without a guard (`tracing::enabled!`, feature flag, or config knob).

> **Hot-path baseline:**
> `#[instrument(skip_all, level = "trace", fields(... cheap only ...))]` + gated/lazy recording for anything expensive, or no instrumentation.

### Readability / Simplicity
- **D. Deeply nested control flow** (more than **2 levels** on hot paths, **3** elsewhere) when it can be flattened with guard clauses, early returns, or `match`.
- **E. Large monolithic functions** (> ~80 lines, or doing multiple responsibilities) when they can be split into focused helpers.
- **F. Clever/opaque iterator chains** that obscure intent or allocate inadvertently; prefer a clear `for` loop or smaller helpers.
- **G. Non-idiomatic error handling:** `unwrap/expect` in non-test code (except proven cold init/CLI); missing context on propagated errors where it matters.
- **H. Naming that hides intent:** cryptic abbreviations, misleading words, type-hiding via wild generics without good reason.
- **I. Unbounded growth patterns:** maps/vectors with unbounded cardinality in hot paths without explicit caps/eviction.
- **J. Unsafe without proof:** `unsafe` blocks without precise safety comments, tests, or measurable wins.

---

## Goals

- **Performance-first:** avoid avoidable runtime overhead (allocations, cloning, stringification, needless async boundaries, dynamic dispatch in tight loops).
- **Readability-always:** shallow control flow, small focused functions, explicit intent, idiomatic Rust.
- **Strict `tracing` rules:** instrumentation must not silently tax hot paths.
- **Ergonomic APIs:** make the correct thing easy and the slow/confusing thing hard.

---

## Default Review Posture

- If code touches a **hot path** (request handling, routing, query planning, JSON/IO, allocators, pools), assume **budget = near zero**.
- If a PR adds logging/tracing/metrics:
  - Confirm **runtime cost ≈ 0** when disabled or below the current level.
  - If cost is non-trivial, **require gating** or relocation to cold paths.
- Prefer **benchmarks/micro-benchmarks** for any plausible p95/p99 or memory impact.
- Enforce **nesting depth limits** and **function size** constraints unless there’s a strong, documented reason.

---

## `tracing` Rules (Allowed vs Banned)

### ✅ Allowed (safe-by-default)
```rust
use tracing::instrument;

#[instrument(skip_all, level = "trace", fields(user_id = %user.id, req_id = %req.id))]
async fn handle(user: &User, req: &Request) -> Result<Response> {
    // Optional event with cheap fields only
    tracing::trace!(%user.id, %req.id, "entered handler");
    Ok(...)
}
