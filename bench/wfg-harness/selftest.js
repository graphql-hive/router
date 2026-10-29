#!/usr/bin/env node
// Checks the checks. Each generated operation has to pass as it is first, then it runs again
// once per defect, a router mistake made on purpose: in the response it sends back, or in what
// it sends a subgraph and gets back. A defect the harness doesn't fail is a blind spot. See README.md.

import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { parseArgs } from 'node:util';

import { parse } from 'graphql';

import { analyzeCase } from './src/analyze.js';
import { createData } from './src/data.js';
import { generate, preferredTypes, prepare } from './src/generated.js';
import { startSession } from './src/session.js';
import { loadSupergraph } from './src/supergraph.js';

const here = import.meta.dirname;
const repo = resolve(here, '../..');

const { values: options } = parseArgs({
  options: {
    only: { type: 'string', multiple: true },
    ops: { type: 'string', default: '10' },
    seed: { type: 'string', default: '0' },
    router: { type: 'string', default: join(repo, 'target/release/hive_router') },
    'dev-cli': { type: 'string', default: join(repo, 'target/release/qp-dev-cli') },
    'subgraph-port': { type: 'string', default: '4220' },
    'router-port': { type: 'string', default: '4221' },
    out: { type: 'string', default: join(here, 'out/selftest') },
  },
});

// fixtures with entity calls in lists, abstract types, `@requires`, conditions and contexts
const fixtures = (
  options.only ?? [
    'tests/abstract-types',
    'tests/requires_requires',
    'tests/union-member-entity-fields',
    'tests/nested-entity-lists',
    'tests/requires-alias-ancestor-same-field',
    'tests/context-multi-hop',
    // faults on `@requires` and context inputs the client never asked for
    'tests/mutations',
    'tests/context-fetch-failures',
    'issues/1308',
    'issues/1309',
  ]
).map((part) => join(repo, 'bin/router/fixture', `${part}.supergraph.graphql`));

// --- what a defect can reach ---

/** Paths to every non-null leaf value of `value`, `__typename` left out. */
function leaves(value, path = [], out = []) {
  if (Array.isArray(value)) value.forEach((item, index) => leaves(item, [...path, index], out));
  else if (value && typeof value === 'object') {
    for (const [key, inner] of Object.entries(value)) if (key !== '__typename') leaves(inner, [...path, key], out);
  } else if (value != null) out.push(path);
  return out;
}

/** Paths to every object of `value` below the root. */
function objects(value, path = [], out = []) {
  if (Array.isArray(value)) value.forEach((item, index) => objects(item, [...path, index], out));
  else if (value && typeof value === 'object') {
    if (path.length) out.push(path);
    for (const [key, inner] of Object.entries(value)) objects(inner, [...path, key], out);
  }
  return out;
}

/** The first object of `value` that isn't a list, lists of lists included. */
const firstObject = (value) =>
  objects(value)
    .map((path) => at(value, path))
    .find((item) => !Array.isArray(item));

const at = (root, path) => path.reduce((value, step) => value?.[step], root);
const put = (root, path, value) => (at(root, path.slice(0, -1))[path.at(-1)] = value);
// the middle one, so the pick is the same every run and not always the first field
const pick = (items) => items[Math.floor(items.length / 2)];

/** Two leaves at the same field of two items of one list, with other values. */
function siblingLeaves(data) {
  const all = leaves(data);
  for (const path of all) {
    const index = path.findLastIndex((step) => typeof step === 'number');
    if (index < 0) continue;
    const other = all.find(
      (candidate) =>
        candidate.length === path.length &&
        candidate[index] !== path[index] &&
        candidate.every((step, i) => i === index || step === path[i]) &&
        at(data, candidate) !== at(data, path),
    );
    if (other) return [path, other];
  }
  return null;
}

/** The representations lists of an `_entities` request, by variable name. */
function representationLists(body) {
  if (!body.query?.includes('_entities')) return [];
  return Object.entries(body.variables ?? {}).filter(
    ([, value]) => Array.isArray(value) && value.length && value.every((item) => item?.__typename),
  );
}

// --- the defects ---
// `response(response)` breaks the router's response, `request`/`result` what a subgraph gets or
// sends back, the first time it can. Each returns whether it found something to break. `mode` is
// the run it's checked in. `expect` is the checks that should catch it, another check failing
// doesn't count. `gap` says why nothing will.

const defects = [
  {
    name: 'wrong value',
    expect: ['data differences'],
    about: 'one leaf value in the response is wrong',
    mode: 'clean',
    response(response) {
      const path = pick(leaves(response.data));
      if (!path) return false;
      put(response.data, path, `${at(response.data, path)}-wrong`);
      return true;
    },
  },
  {
    name: 'missing value',
    expect: ['data differences'],
    about: 'one leaf comes back null, like a fetch that never ran',
    mode: 'clean',
    response(response) {
      const path = pick(leaves(response.data));
      if (!path) return false;
      put(response.data, path, null);
      return true;
    },
  },
  {
    name: 'swapped between entities',
    expect: ['data differences'],
    about: 'a field of one list item ends up on another',
    mode: 'clean',
    response(response) {
      const pair = siblingLeaves(response.data);
      if (!pair) return false;
      const [left, right] = pair;
      const value = at(response.data, left);
      put(response.data, left, at(response.data, right));
      put(response.data, right, value);
      return true;
    },
  },
  {
    name: 'error on a value',
    expect: ['unexpected errors'],
    about: 'an error pointing at a value the client got',
    mode: 'clean',
    response(response) {
      const path = pick(leaves(response.data));
      if (!path) return false;
      (response.errors ??= []).push({ message: 'wrong', path });
      return true;
    },
  },
  {
    name: 'request error',
    expect: ['request errors'],
    about: 'an error without a path next to the data',
    mode: 'clean',
    response(response) {
      if (!response.data) return false;
      (response.errors ??= []).push({ message: 'wrong' });
      return true;
    },
  },
  {
    name: 'errors dropped',
    expect: ['errors missing'],
    // a hidden fault gives router errors the monolith has nothing to compare with
    hiddenGap: true,
    about: 'the router loses every error a failed field gave',
    mode: 'faults',
    response(response) {
      if (!response.errors?.length) return false;
      delete response.errors;
      return true;
    },
  },
  {
    name: 'error moved to an object',
    expect: ['unexpected errors'],
    // a hidden fault gives router errors the monolith has nothing to compare with
    hiddenGap: true,
    about: "an error's path points at another object than the one that failed",
    mode: 'faults',
    response(response) {
      const error = response.errors?.find((candidate) => candidate.path);
      const elsewhere = objects(response.data).find((path) => path.join('.') !== error?.path.join('.'));
      if (!error || !elsewhere) return false;
      error.path = elsewhere;
      return true;
    },
  },
  {
    name: 'wrong value, with faults',
    expect: ['data differences'],
    about: 'one leaf value is wrong while some fields fail',
    mode: 'faults',
    response(response) {
      const path = pick(leaves(response.data));
      if (!path) return false;
      put(response.data, path, `${at(response.data, path)}-wrong`);
      return true;
    },
  },
  {
    name: 'unrelated null, with faults',
    expect: ['data differences'],
    about: 'a value nowhere near a failed field comes back null while some fields fail',
    mode: 'faults',
    response(response) {
      const errorPaths = (response.errors ?? []).filter((error) => error.path).map((error) => error.path.join('.'));
      const related = (a, b) => a === b || a.startsWith(`${b}.`) || b.startsWith(`${a}.`);
      const path = pick(leaves(response.data).filter((candidate) => !errorPaths.some((errorPath) => related(candidate.join('.'), errorPath))));
      if (!path) return false;
      put(response.data, path, null);
      return true;
    },
  },
  {
    name: 'duplicate representation',
    expect: ['duplicate-representation'],
    about: 'the router sends one entity twice in one list',
    mode: 'clean',
    request(graph, body) {
      const [entry] = representationLists(body);
      if (!entry) return false;
      entry[1].push(structuredClone(entry[1][0]));
      return true;
    },
  },
  {
    name: 'representations swapped',
    expect: ['data differences', 'wrong-representation'],
    // the router can fetch the same field of the same entity again elsewhere and write the right
    // value over it, then the client never sees the defect
    changesData: true,
    about: 'the router puts entity results back on the wrong objects',
    mode: 'clean',
    request(graph, body) {
      const entry = representationLists(body).find(
        ([, list]) => list.length > 1 && JSON.stringify(list[0]) !== JSON.stringify(list[1]),
      );
      if (!entry) return false;
      [entry[1][0], entry[1][1]] = [entry[1][1], entry[1][0]];
      return true;
    },
  },
  {
    name: 'wrong key',
    expect: ['data differences', 'wrong-representation'],
    // the router can fetch the same field of the same entity again elsewhere and write the right
    // value over it, then the client never sees the defect
    changesData: true,
    about: 'a representation carries another entity of the same type',
    mode: 'clean',
    request(graph, body) {
      const [entry] = representationLists(body);
      const representation = entry?.[1][0];
      const key = representation && Object.keys(representation).find((field) => typeof representation[field] === 'string' && /:\d+$/.test(representation[field]));
      if (!key) return false;
      representation[key] = representation[key].replace(/\d+$/, (seed) => String(Number(seed) + 1));
      return true;
    },
  },
  {
    name: 'entity call answered empty',
    expect: ['data differences'],
    // the router can fetch the same field of the same entity again elsewhere and write the right
    // value over it, then the client never sees the defect
    changesData: true,
    about: 'a subgraph call that should have run gives nothing back',
    mode: 'clean',
    result(graph, body, result) {
      if (!representationLists(body).length || !result.data) return false;
      for (const key of Object.keys(result.data)) {
        if (Array.isArray(result.data[key])) result.data[key] = result.data[key].map(() => null);
      }
      return true;
    },
  },
  {
    name: 'value fetched for nothing',
    expect: ['unused values'],
    about: 'a subgraph sends a value nobody reads, a field fetched for nothing',
    mode: 'clean',
    result(graph, body, result) {
      const entity = firstObject(result.data);
      if (!entity) return false;
      entity.extraField = `${entity.__typename ?? 'Object'}.extraField:selftest`;
      return true;
    },
  },
  {
    name: 'value fetched twice',
    expect: ['unused values'],
    about: 'a subgraph sends a value again that the client already gets from elsewhere',
    mode: 'clean',
    // values are unique per place, the same one twice counts as used once it's used anywhere
    gap: 'the unused check goes by value, not by where it was fetched',
    result(graph, body, result) {
      const entity = firstObject(result.data);
      const used = entity && leaves(entity).map((path) => at(entity, path)).find((value) => typeof value === 'string');
      if (!used) return false;
      entity.extraField = used;
      return true;
    },
  },
];

// --- the representation exemption, directly ---
// Under a hidden fault, a representation that doesn't decode is excused only when nulls the failed
// input explains are all that's wrong with its key. Few generated operations get there, so these
// are built by hand, on the fixture where it happens: the key is `searchCriteria { partialCriteria }`,
// `bestOffer` requires `searchCriteria { fullCriteria }`, and `fullCriteria` failed.

const exemptionCases = (() => {
  const supergraph = loadSupergraph(
    readFileSync(join(repo, 'bin/router/fixture/tests/nested-key-requires-reentry.supergraph.graphql'), 'utf8'),
  );
  const data = createData(supergraph);
  const typeName = 'ProductSearchResultResponse';
  const excused = (representation) => {
    const log = [
      {
        graph: 'a',
        result: { errors: [{ message: 'SearchCriteria.fullCriteria failed' }] },
        issues: [
          {
            kind: 'undecodable-representation',
            detail: JSON.stringify(representation),
            causes: data.whyUndecodable(typeName, { __typename: typeName, ...representation }),
          },
        ],
        lookups: [],
      },
    ];
    const analysis = analyzeCase({
      response: { data: {} },
      expected: { data: {} },
      log,
      switchedOff: new Set(),
      inputField: supergraph.inputField,
      requiresUnder: supergraph.requiresUnder,
    });
    return !analysis.issues['undecodable-representation'];
  };
  return [
    ['the failed input nulled the key object', { searchCriteria: null }, true],
    ['the failed input nulled, next to a broken key value', { searchCriteria: { partialCriteria: 'broken', fullCriteria: null } }, false],
    ['a key field missing', {}, false],
    ['a null the failed input doesn\'t explain', { searchCriteria: { partialCriteria: null } }, false],
  ].map(([name, representation, expected]) => ({ name, ok: excused(representation) === expected, expected }));
})();

// --- run ---

const rows = defects.map((defect) => ({ defect, applied: 0, caught: 0, by: {}, missed: [] }));
let baseline = { ran: 0, usable: 0 };

for (const fixture of fixtures) {
  const name = fixture.split('/').slice(-2).join('/');
  const session = await startSession({
    sdl: prepare(readFileSync(fixture, 'utf8')),
    router: options.router,
    subgraphPort: Number(options['subgraph-port']),
    routerPort: Number(options['router-port']),
    outDir: join(options.out, name.replace('/', '-')),
  });
  const operations = generate(
    options['dev-cli'],
    join(options.out, name.replace('/', '-'), 'supergraph.graphql'),
    Number(options.ops),
    Number(options.seed),
  );

  for (const operation of operations) {
    const document = parse(operation.document);
    session.data.variant.preferredTypes = preferredTypes(document);
    const run = async (mode, defect) => {
      session.data.variant.faults = mode === 'faults';
      let applied = false;
      const once = (hook) => (...args) => {
        if (!applied && hook(...args)) applied = true;
      };
      session.subgraphHooks.request = defect?.request && once(defect.request);
      session.subgraphHooks.result = defect?.result && once(defect.result);
      const result = await session.runCase({
        operationText: operation.document,
        document,
        operationName: operation.operationName,
        variables: operation.variables,
        salt: operation.seed,
        mutateResponse: defect?.response && once(defect.response),
      });
      session.subgraphHooks.request = session.subgraphHooks.result = undefined;
      return { applied, result };
    };

    // only operations that pass as they are say anything about a defect
    baseline.ran++;
    const clean = await run('clean');
    const faults = await run('faults');
    // faults runs don't count unused values, same as fuzz.js
    const problemsIn = (mode, result) =>
      (result.problems ?? []).filter((problem) => mode === 'clean' || !problem.endsWith('unused values'));
    const passes = (mode, { result }) => result.response?.data != null && !problemsIn(mode, result).length;
    if (!passes('clean', clean)) continue;
    baseline.usable++;

    for (const row of rows) {
      if (row.defect.mode === 'faults' && !passes('faults', faults)) continue;
      const { applied, result } = await run(row.defect.mode, row.defect);
      if (!applied) continue;
      // the client's data can't show it then, but what the router sent the subgraph still can
      if (row.defect.changesData && JSON.stringify(result.response?.data) === JSON.stringify(clean.result.response.data)) {
        row.noEffect = (row.noEffect ?? 0) + 1;
        if (Object.keys(result.analysis?.issues ?? {}).length) row.noEffectFlagged = (row.noEffectFlagged ?? 0) + 1;
        continue;
      }
      // with faults, the case is only meaningful where the harness didn't stand down for a
      // fault the monolith never sees
      const hidden = result.analysis?.hiddenFaults?.length > 0;
      const key = row.defect.mode === 'faults' && hidden ? 'hiddenFault' : 'applied';
      row[key] = (row[key] ?? 0) + 1;
      const problems = problemsIn(row.defect.mode, result);
      const kinds = problems.map((problem) => problem.replace(/^\d+\+? /, ''));
      if (kinds.some((kind) => row.defect.expect.includes(kind))) {
        row[key === 'applied' ? 'caught' : 'caughtHidden'] = (row[key === 'applied' ? 'caught' : 'caughtHidden'] ?? 0) + 1;
        for (const problem of problems) {
          const kind = problem.replace(/^\d+\+? /, '');
          row.by[kind] = (row.by[kind] ?? 0) + 1;
        }
      } else if (row.missed.length < 3) {
        row.missed.push(`${name} seed-${operation.seed}${hidden ? ' (hidden fault)' : ''}`);
      }
    }
  }
  await session.shutdown();
}

console.log(`${baseline.usable} of ${baseline.ran} operations passed as they were and were used\n`);
let blind = 0;
console.log('representation exemption:');
for (const check of exemptionCases) {
  console.log(`${check.ok ? '✓' : '✗'} ${check.name}: ${check.expected ? 'excused' : 'caught'}${check.ok ? '' : ', but it was not'}`);
}
if (exemptionCases.some((check) => !check.ok)) blind++;
console.log('');
for (const row of rows) {
  const { defect } = row;
  const share = row.applied || row.hiddenFault ? `${row.caught}/${row.applied}` : 'never applied';
  const hidden =
    (row.hiddenFault ? `, ${row.caughtHidden ?? 0}/${row.hiddenFault} with a hidden fault` : '') +
    (row.noEffect ? `, ${row.noEffect} no effect (${row.noEffectFlagged ?? 0} flagged by the subgraph checks)` : '');
  const by = Object.entries(row.by)
    .sort((a, b) => b[1] - a[1])
    .map(([kind, count]) => `${kind} ${count}`)
    .join(', ');
  const missedHidden = !defect.hiddenGap && (row.caughtHidden ?? 0) < (row.hiddenFault ?? 0);
  const missedAll = (row.applied && row.caught < row.applied) || missedHidden;
  if (missedAll && !defect.gap) blind++;
  // nothing to say when it was never applied where it can be judged
  const untested = !row.applied && (defect.hiddenGap || !row.hiddenFault);
  if (untested) blind++;
  const mark = untested ? '?' : !missedAll ? '✓' : defect.gap ? '~' : '✗';
  console.log(`${mark} ${defect.name.padEnd(28)} ${(share + hidden).padEnd(36)} ${by}`);
  if (missedAll) console.log(`    missed: ${row.missed.join(', ')}${defect.gap ? ` (known: ${defect.gap})` : ''}`);
}
console.log(`\n✓ always caught, ✗ missed sometimes, ~ a known blind spot, ? never tested (fails the run)`);
if (!baseline.usable) console.log('no operation passed as it was, nothing was tested');
process.exit(blind || !baseline.usable ? 1 : 0);
