# Router harness

Runs operations through the router against mocked subgraphs with deterministic data, and checks:

- the response `data` equals a monolith's, executing the operation directly over the same data
- the router's errors point at data that's null or missing, and there are some when the monolith
  has errors
- no subgraph is asked for a field it doesn't resolve (`not-owned`), `@provides` included
- every representation decodes to an entity and carries that entity's real key and `@requires`
  values (`undecodable-representation`, `wrong-representation`)
- no entity is sent without anything asked of it (`empty-lookup`), or twice in one list
  (`duplicate-representation`)
- every `String`/`ID` the subgraphs return ends up in the response or in a later subgraph request
  (`unused`). Keys and `@requires` inputs of entity calls the variables switch off don't count,
  read from the plan the router exposes: the parent fetch selects them unconditionally

It also counts requests per subgraph, entity lookups, and entities looked up more than once in a
subgraph (`repeated`), which isn't wrong in itself but is worth a look.

Two runners, and a check of the checks:

- `run.js`: the WFG operations (`bin/router/fixture/wfg`), with the `@include`/`@skip` variables
  set every way
- `fuzz.js`: random operations (the `bin/differential` generator) against every fixture supergraph
  under `bin/router/fixture`, directories included, except the ones `known.json` excludes. Each
  operation runs twice: clean, and with faults, where some nullable fields are null and some leaf
  fields fail. The faults run doesn't count unused values, a failed non-null field takes what was
  fetched under it along. A fault is hidden when the field is a `@requires` or context input and
  the monolith never reported it: the router fetched it for itself, the monolith never resolves
  it. Then the router's data may be null where the monolith's isn't, as long as one of its errors
  about a hidden fault is at, above or below that null; those errors may sit on data the client
  got; and a representation may not decode when nulls in its key are all that's wrong with it and
  each is where a `@requires` of the entity asks for the failed field, at it or above it. The
  self-test checks that one directly, with a broken key value next to such a null among others. Every other difference still counts
- `selftest.js`: breaks things on purpose and checks the harness notices. Each generated operation
  has to pass as it is first, then runs once per defect: a wrong, missing or swapped value, an error
  on data the client got or moved elsewhere, errors dropped, a duplicate, swapped or wrong
  representation, an entity call answered empty, a value fetched for nothing. It prints how often
  each was caught and by which check; only the checks a defect names count as catching it. It
  exits with 1 when one slipped through that isn't a known blind spot, when one was never tested
  (`?`), or when no operation passed as it was. A defect that didn't change what the client got, because another call fetched the
  same field again, counts as no effect, with what the subgraph checks said about it

## Run

```bash
cargo build --release -p hive-router --bin hive_router -p qp-dev-cli --bin qp-dev-cli
cd bench/wfg-harness
node run.js --operation query-2
node run.js --operation query-1 --cases full
node fuzz.js --ops 20
node fuzz.js --only context --only requires --ops 50
node selftest.js --ops 20
```

`run.js` options: `--operation` (`query-1`, `query-2`, `mutation-1` or a path), `--router` (binary,
`target/release/hive_router` by default), `--cases quick|full`, `--random <n>`,
`--subgraph-port`, `--router-port`, `--out`, `--logs failed|all|none`, `--keep-scalars`.

`fuzz.js` options: `--only <part of a fixture path>` (repeatable), `--ops <n>` per fixture,
`--seed <first>`, `--no-faults`, `--router`, `--dev-cli` (`target/release/qp-dev-cli`, which
generates the operations), `--subgraph-port`, `--router-port`, `--out`.

`run.js` exits with 1 when a case fails. It writes `out/report.json`, and `out/<case>.log.json`
with the router's response, the monolith's, and every subgraph request with its response, for the
failing cases only unless `--logs all`.

Every `fuzz.js` operation ends up as one of:

- `passed`
- `failed`: a check failed, clean or with faults
- `planner-refused`: the router couldn't plan it (`QUERY_PLAN_BUILD_FAILED`), with the planner's
  error from the router log
- `harness-error`: the harness itself threw
- `invalid-rejected`: the generated operation isn't valid GraphQL (graphql-js against the same
  schema), and the router's validation rejected it (`GRAPHQL_VALIDATION_FAILED`). Counted apart,
  it says nothing about execution. An invalid operation the router runs, or fails on later (an
  internal planner error, say), is `failed`

and a fixture can be `skipped` (excluded, it doesn't load, the router doesn't start, or there
are no operations). A fixture can also get fewer operations than `--ops`: `generate` keeps only
operations that pass validation, and gives up after 50 seeds per operation (heavy-query gets 8 of
20, context-basic 18). Each fixture has `requested`, `generated` and `shortfall` in
`summary.json`, a short one says so on its line, and a last line totals them, apart from the
outcomes: 8 operations that pass are 8 operations of coverage, not 20. A shortfall doesn't fail
the run. Fixtures that didn't get as far as generating are counted as not run instead. `fuzz.js` exits with 1, and prints the fixtures, unless every operation passed
or is known. [`known.json`](known.json) has, by fixture path, with why: `excluded` (not run at
all), `skipped` with the `skipError` it's skipped with, and `planner-refused` with the
`plannerErrors` it may refuse with (none without it). A refusal another federation planner
can plan doesn't belong there, it's a planner gap. It writes `out/fuzz/summary.json`, with every operation's
outcome by seed (`cases`), and `out/fuzz/<fixture>/seed-<n>[-faults].json` with the operation,
the responses and the subgraph requests of every one that didn't pass. A run first removes the
`seed-*` files a run into the same directory left.

`node compare.js out/a/report.json out/b/report.json` puts two `run.js` runs side by side, like the
router of `main` against this branch. `node compare.js out/fuzz/a out/fuzz/b` compares two fuzz
runs by their `cases`: the operations whose outcome changed, and the ones only one of them ran, as
not run, never as fixed. Seeds are only the same operation when both runs used the same `--seed`
and the same generator.

`graphql` resolves from `bench/node_modules`, or `npm install` here.

## How the data works

A value is a pure function of where it lives, so the subgraphs agree on it without sharing
anything, and the monolith gives the expected response:

- an object's seed comes from its parent's seed, the field, the arguments and the list index
- an entity's key fields carry its seed (`Type:seed`, the seed itself for an `Int`), so a subgraph
  gets the same entity from a representation and can check the rest of it. The type in the key is
  how an `@interfaceObject` or entity interface representation gets back to its object type, and
  a nested key object that's an entity itself is that entity under the same seed
- a `String`/`ID` that isn't a key is `Parent.field:hash`, unique, which is what the `unused`
  check relies on
- a field that `@requires` others depends on their values, a subgraph reads them from the
  representation, the monolith from the entity. A wrong one shows in the data
- a `@fromContext` argument is part of the value too, the router passes it, the monolith reads it
  from the nearest object that sets the context (a context on an interface or a union is set by
  every type behind it)
- an abstract field returns a type every subgraph resolving it knows, `null` when there's none,
  and never an `@inaccessible` one. A list of an abstract type has an item of every such type, so
  every type condition gets data
- lists have 2 items near the root and 1 below, so the response stays small
- every field that returns an `Int`, `Float` or `Boolean` returns a `String` instead, in the
  supergraph both the router and the monolith get (1399 fields in WFG, `--keep-scalars` to keep
  them). Those values are unique then too: in query-1 the `unused` check sees 81% of what the
  subgraphs return instead of 64%. Arguments and input fields keep their types

## Limits

- enum values and `null`s aren't unique, the `unused` check doesn't see them
- the `unused` check goes by value: a value fetched twice counts as used once either copy is. The
  same field of the same entity fetched at two places isn't a failure, it's counted as `repeated`
- errors are checked for where they point (a null, or below one, or an object of the type a failed
  `@requires` or context input belongs to, by `__typename` or by its own values) and that there are some when the monolith has any, not one by one
- under a fault the monolith never sees, a wrong null next to one of the router's errors isn't
  caught, nor a missing or misplaced error (the monolith has none to compare with)
- a hidden fault is matched by its field's name: where a `@requires` asks for a field of that name,
  and which errors are about it. Two failures of the same field aren't told apart
- the representation checks see whether a representation is a real entity with the right key and
  `@requires` values, not whether it's the entity that belongs at that place: two swapped
  representations only show in the data. When another call writes the right value over it, that
  doesn't show either
- a field reached through an interface counts as resolved by a subgraph when the interface's field
  is, WFG has types without a `join__field` for fields of an interface they implement there
- an `@interfaceObject` subgraph answers with the object types, not the interface, so the router
  never has to find out the type of one
- `@authenticated`, `@requiresScopes` and `@policy` are taken out of the supergraph
- variables are generated: by the generator in `fuzz.js`, from the types in `run.js` (the first
  enum value, required input fields, optional ones only at the top)
