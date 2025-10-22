# Gemini Code Assistant - Style Guide (Performance‑First)

> TL;DR: Treat performance as a feature. Prefer zero/low‑overhead code paths, avoid accidental allocations, and never pay logging costs on hot paths unless explicitly gated.

---

## Quick Facts (Tracing Semantics You Must Know)

- **Default `#[instrument]` level is `debug`.**
- Tracing levels are ordered: `error > warn > info > debug > trace`. Enabling `info` **does not** enable `debug`.
- **Field expressions inside `#[instrument(... fields(...))]` are evaluated at function entry** (macro expansion time for that call site), before any subscriber decides to record or drop the span. If those expressions do heavy work (e.g., `pretty_print`, `format!`, `to_string`, serialization), you **pay that cost** regardless of whether the span is ultimately recorded.
- Therefore, any non‑trivial work inside `fields(...)` is a foot‑gun even when you think "it's only at `debug`."

---

## Blocking Policy (exact)

Gemini must **request changes (block the PR)** when any of the following appear:

- **A. `#[tracing::instrument]` uses the default level (`info`) on hot paths.**
  `info` spans are commonly enabled in production. Span creation + any eager field evaluation will run on hot paths. Use `level = "trace"` for hot code and record richer details lazily (see patterns below) when a higher level is actually enabled.

- **B. `#[tracing::instrument]` uses `level = "debug"` (or higher verbosity) and either condition holds:**
  1) **`skip_all` is missing**, or
  2) **`fields(...)` performs any computation** (function/method calls, formatting, allocation, serialization, cloning, traversals).
  Even if the runtime filter disables that `debug` span, those field expressions are still evaluated at function entry - wasted work.

- **C. Any heavy work is performed just to log/trace on a hot path** without a guard (e.g., `tracing::enabled!` or a feature flag).

> **Baseline for hot paths:** `#[instrument(skip_all, level = "trace", fields(... cheap only ...))]` + gated/lazy recording for anything expensive or no instrumentation at all.

---

## Goals

- **Always review PRs through a performance lens.**
- **Block** changes that add avoidable runtime overhead (allocations, stringification, extra clones, unnecessary async/await boundaries, unbounded maps/vecs, etc.).
- **Strict rules for `tracing`** so instrumentation doesn't silently tax hot paths.
- **Review PRs through a "best-practices in Rust" lens.**

---

## Default Review Posture

- If the change touches a **hot path** (request handling, routing, query planning, JSON/IO, allocators, pools), assume **budget = near zero**.
- If a PR adds logging/tracing/metrics:
  - Confirm **runtime cost ≈ 0** when disabled or below the current level.
  - If cost is non‑trivial, **require gating** (`tracing::enabled!`, feature flags) or relocation to cold paths.
- Prefer **benchmarks or micro‑benchmarks** for anything that plausibly affects p95/p99 latency or memory.

---

## `tracing` Rules (Allowed vs Banned)

### ✅ Allowed (safe‑by‑default)

```rust
use tracing::instrument;

#[instrument(skip_all, level = "trace", fields(user_id = %user.id, req_id = %req.id))]
async fn handle(user: &User, req: &Request) -> Result<Response> {
    // Optional event with cheap fields only
    tracing::trace!(%user.id, %req.id, "entered handler");
    Ok(...)
}
```

- `level = "trace"` on `instrument` for hot paths.
- `skip_all` + **explicit, cheap fields** (`%`/`?` over already‑available, small values).
- No allocations or heavy computation in attribute arguments.

### ❌ Banned (examples)

```rust
// 1) Default level (info) + no skip_all.
#[instrument] // reject
fn hot_path(a: &Big, b: &Bigger) { /* ... */ }

// 2) Debug‑level span with expensive field computation.
#[instrument(level = "debug", fields(details = %self.pretty_print()))] // reject
fn parse(&self) { /* ... */ }

// 3) Any format/to_string/clone/serialize in fields
#[instrument(skip_all, level="info", fields(blob = %format!("{:?}", self.blob)))] // reject
async fn foo(self) { /* ... */ }
```

---

## Patterns for Safe, Lazy Recording

When you truly need expensive fields, **gate them** and **record lazily**:

### Record after `enabled!` check

```rust
use tracing::{self, Level};

#[instrument(skip_all, level = "trace", fields(details = tracing::field::Empty))]
fn process(&self) {
    if tracing::enabled!(Level::DEBUG) {
        // Compute only when actually enabled at runtime.
        let details = self.pretty_print(); // expensive
        tracing::Span::current().record("details", &tracing::field::display(details));
    }
    // ...
}
```

---

## Field Hygiene

- **Cheap only:** ids, small scalars, already‑borrowed references.
- **Forbidden in fields:** `.pretty_print()`, `format!`, `.to_string()`, (de)serialization, cloning big structs, traversing collections.
- Prefer `fields(foo = %id)` over `fields(foo = id.to_string())`.

---

## Reviewer Checklist (what Gemini should look for)

- [ ] Any `#[instrument]`?
  - [ ] Uses `skip_all`? If **no** → **request changes**.
  - [ ] Uses `level = "trace"` for hot paths? If default or `level = "info"` on hot code → **request changes**.
  - [ ] **No function calls/allocs** inside `fields(...)`? If present → **request changes** with Pattern A/B.
- [ ] New logs/metrics in hot paths?
  - [ ] Gated via `enabled!` or a feature flag?
- [ ] New `clone()` / `to_string()` / `collect()` on hot paths? Ask for justification or refactor.
- [ ] Async boundaries added? Avoid splitting critical sections if not needed.
- [ ] Allocations: prefer stack/arena/borrowed views; avoid intermediate `String`/`Vec` unless necessary.
- [ ] Config toggles: if a costly path is optional, require a **feature flag** or runtime knob.

---

## Comment Templates (Gemini → PR)

**1) Instrumentation level & missing `skip_all`**

> Performance check: `#[instrument]` here uses the default level (`info`) and/or doesn't specify `skip_all`.
> This creates spans at common prod filters and may evaluate fields eagerly.
> Please switch to:
> ```rust
> #[instrument(skip_all, level = "trace", fields(... only cheap fields ...))]
> ```
> If you need richer context, record it after an `enabled!` guard.

**2) Expensive field computation**

> The `fields(...)` contains an eager computation (`pretty_print`/`format!`/`to_string`).
> These are evaluated at function entry even if the span isn't ultimately recorded.
> Please either:
> - Gate with `tracing::enabled!(Level::DEBUG)` + `Span::current().record(...)`, or
> - Use a lazy wrapper and still gate if expensive (see “Patterns for Safe, Lazy Recording”).

**3) Hot‑path logging**

> New logs on a hot path detected. Can we gate by level/feature or move to a colder edge?
> Aim for zero cost when disabled.

---

## Quick‑Fix Snippets Gemini Can Suggest

- Replace:
  ```rust
  #[instrument]
  ```
  With:
  ```rust
  #[instrument(skip_all, level = "trace")]
  ```

- Replace (eager fields):
  ```rust
  #[instrument(skip_all, level="debug", fields(details = %self.pretty_print()))]
  ```
  With (gated record):
  ```rust
  #[instrument(skip_all, level="trace", fields(details = tracing::field::Empty))]
  fn f(&self) {
      if tracing::enabled!(tracing::Level::DEBUG) {
          let d = self.pretty_print();
          tracing::Span::current().record("details", &tracing::field::display(d));
      }
  }
  ```

---

## When It's Okay to Use `info`

- Truly **cold** admin/maintenance paths (schema reload, health checks, CLI tools) **may** use `level = "info"` with **cheap** fields only.
- Still **avoid** expensive field computation in attributes; prefer gated record.

---

## Rationale (why we're strict)

- `#[instrument]` **creates spans** and **evaluates its `fields(...)`** at function entry. With the default `info` level, this happens under typical production filters; with `debug`, the field expressions are still evaluated even if the span is dropped later.
- Stringification/pretty printing can dominate latency on tight loops; a handful of these across hot paths quickly becomes a measurable tax.
- The safe baseline (`skip_all`, `level="trace"`, gated recording) keeps the lights on without sacrificing debuggability when you need it.

---

## PR Author Checklist (pre‑submit)

- [ ] No default/`info`/`debug` `#[instrument]` on hot paths unless justified and still `skip_all` + cheap fields only.
- [ ] Every `#[instrument]` has `skip_all`.
- [ ] No function calls/allocs in `fields(...)`.
- [ ] Hot‑path logs/metrics are gated or relocated.

---
