#!/usr/bin/env node
// Random operations against every fixture supergraph, each one checked like the WFG operations
// are: the router's response against the monolith's, what the subgraphs were asked, and what of
// it was used. With faults on, some fields are null and some fail. See README.md.

import { mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { parseArgs } from 'node:util';

import { parse, validate } from 'graphql';

import { startSession } from './src/session.js';
import { generate, outName, preferredTypes, prepare } from './src/generated.js';

const here = import.meta.dirname;
const repo = resolve(here, '../..');

const { values: options } = parseArgs({
  options: {
    // a fixture path, or part of one
    only: { type: 'string', multiple: true },
    ops: { type: 'string', default: '20' },
    // each operation also runs with this many more random sets of its Boolean variables
    assignments: { type: 'string', default: '0' },
    seed: { type: 'string', default: '0' },
    // some nullable fields null, some leaf fields failing
    faults: { type: 'boolean', default: true },
    'no-faults': { type: 'boolean', default: false },
    router: { type: 'string', default: join(repo, 'target/release/hive_router') },
    'dev-cli': { type: 'string', default: join(repo, 'target/release/qp-dev-cli') },
    'subgraph-port': { type: 'string', default: '4220' },
    'router-port': { type: 'string', default: '4221' },
    out: { type: 'string', default: join(here, 'out/fuzz') },
  },
});
const faults = options.faults && !options['no-faults'];

/** `count` copies of `operation` with its Boolean variables set at random, one left out now and
 * then when it has a default. Seeded by the operation's seed, so a run repeats. */
function variants(operation, count) {
  const definitions = parse(operation.document).definitions.find((d) => d.kind === 'OperationDefinition')?.variableDefinitions ?? [];
  const booleans = definitions.filter((d) => d.type.kind === 'NonNullType' && d.type.type.name?.value === 'Boolean');
  if (!booleans.length) return [];
  let state = (operation.seed * 2654435761) >>> 0 || 1;
  const random = () => ((state = (state * 1103515245 + 12345) >>> 0), state / 2 ** 32);
  return Array.from({ length: count }, (_, k) => {
    const variables = { ...operation.variables };
    for (const d of booleans) {
      const name = d.variable.name.value;
      if (d.defaultValue && random() < 0.2) delete variables[name];
      else variables[name] = random() < 0.5;
    }
    return { ...operation, seed: operation.seed * 1000 + k + 1, variables };
  });
}

const fixtureRoot = join(repo, 'bin/router/fixture');
// every supergraph under it, in directories too (`bfg/supergraph.graphql`)
const fixtures = readdirSync(fixtureRoot, { recursive: true })
  .filter((path) => path.endsWith('.graphql') && basename(path).includes('supergraph'))
  .map((path) => join(fixtureRoot, path))
  .filter((path) => !options.only || options.only.some((part) => path.includes(part)))
  .sort();

const outDir = resolve(options.out);
mkdirSync(outDir, { recursive: true });
// failures of an earlier run into the same directory would still look like failures to
// compare.js. Other runs nested in here (`out/fuzz/main/<fixture>`) are a level deeper, they stay
for (const entry of readdirSync(outDir, { withFileTypes: true })) {
  if (!entry.isDirectory()) continue;
  for (const file of readdirSync(join(outDir, entry.name))) {
    if (/^seed-\d+(-faults)?\.json$/.test(file)) rmSync(join(outDir, entry.name, file));
  }
}
// The router gives each validation error its rule's code (`OverlappingFieldsCanBeMerged`), from
// the rules in `lib/graphql-tools`. `GRAPHQL_VALIDATION_FAILED` is only the stage's code
const rulesDir = join(repo, 'lib/graphql-tools/src/validation/rules');
const validationCodes = new Set([
  'GRAPHQL_VALIDATION_FAILED',
  ...readdirSync(rulesDir).flatMap((file) =>
    [...readFileSync(join(rulesDir, file), 'utf8').matchAll(/fn error_code\(&self\) -> &'static str \{\s*"([A-Za-z]+)"/g)].map(
      (match) => match[1],
    ),
  ),
]);

// Fixtures that are known to be skipped or to have operations the router can't plan, and why.
// Anything else skipped or refused fails the run, same as a failed case
const known = JSON.parse(readFileSync(join(here, 'known.json'), 'utf8'));

/**
 * The planner error behind a refusal, from what the router logged after `offset`. The response only
 * says "Internal server error".
 */
function lastPlannerError(routerLog, offset) {
  const lines = readFileSync(routerLog, 'utf8').slice(offset).trim().split('\n').reverse();
  for (const line of lines) {
    if (!line.includes('Failed to produce a plan')) continue;
    try {
      return JSON.parse(line).error;
    } catch {
      return line;
    }
  }
  return 'unknown, see router.log';
}

/** Checks a fixture's outcomes against what's known for it, and prints its line. */
function finish(row) {
  // what's allowed for this fixture: known skips and refusals, nothing else
  const expected = known[row.fixture] ?? {};
  row.unexpected = [];
  // a known skip has to be for the known reason
  if (row.skipped && !(expected.excluded || (expected.skipped && row.skipped.includes(expected.skipError)))) {
    row.unexpected.push('skipped');
  }
  // a known fixture can still refuse something new, `plannerErrors` says what it may refuse with
  const refusals = Object.values(row.refusals ?? {});
  const allowed = (error) => expected['planner-refused'] && (expected.plannerErrors ?? []).some((part) => error.includes(part));
  row.knownRefusals = refusals.filter(allowed).length;
  if (row.knownRefusals < refusals.length) row.unexpected.push('planner-refused');
  for (const outcome of ['failed', 'harness-error']) if (row.outcomes[outcome]) row.unexpected.push(outcome);

  const counts = Object.entries(row.outcomes)
    .map(([outcome, count]) => `${count} ${outcome}`)
    .join(', ');
  // fewer operations than asked for is less coverage, not a failure, so it's only said
  const short = row.shortfall ? ` (${row.generated} of ${row.requested} generated, ${row.shortfall} short)` : '';
  const status = row.skipped ? `skipped (${row.skipped})${short}` : `${row.ran} ops${short}: ${counts || 'none'}`;
  const reasons = [expected.excluded, expected.skipped, expected['planner-refused']].filter(Boolean);
  const knownNote = reasons.length && !row.unexpected.length && (row.skipped || refusals.length) ? ` (known: ${reasons.join('; ')})` : '';
  const details = row.unexpected.length && Object.keys(row.problems).length
    ? ` | ${Object.entries(row.problems).map(([kind, count]) => `${count}x ${kind}`).join(', ')}`
    : '';
  console.log(`${row.unexpected.length ? '✗' : ' '} ${row.fixture.padEnd(60)} ${status}${knownNote}${details}`);
}

const summary = [];

for (const fixture of fixtures) {
  const fixtureName = fixture.slice(fixtureRoot.length + 1);
  // by path, several are called `supergraph.graphql`
  const fixtureOut = join(outDir, outName(fixtureName));
  // `cases` has every operation that ran, by seed, with its outcome: passed, failed,
  // planner-refused or harness-error. compare.js goes by it
  const row = { fixture: fixtureName, ran: 0, outcomes: {}, problems: {}, cases: {} };
  summary.push(row);

  if (known[fixtureName]?.excluded) {
    row.skipped = 'excluded';
    finish(row);
    continue;
  }
  let sdl;
  try {
    sdl = prepare(readFileSync(fixture, 'utf8'));
  } catch (error) {
    row.skipped = `can't read: ${error.message.split('\n')[0]}`;
    finish(row);
    continue;
  }
  let session;
  try {
    session = await startSession({
      sdl,
      router: options.router,
      subgraphPort: Number(options['subgraph-port']),
      routerPort: Number(options['router-port']),
      outDir: fixtureOut,
    });
  } catch (error) {
    row.skipped = `session didn't start: ${error.message.split('\n')[0]}`;
    finish(row);
    continue;
  }

  let operations = [];
  row.requested = Number(options.ops);
  try {
    operations = generate(options['dev-cli'], join(fixtureOut, 'supergraph.graphql'), Number(options.ops), Number(options.seed));
  } catch (error) {
    row.skipped = `no operations: ${error.message}`;
  }
  if (!operations.length) row.skipped ??= 'no operations';
  // `generate` keeps only valid operations and gives up after 50 seeds for each
  row.generated = operations.length;
  row.shortfall = row.requested - row.generated;
  // the extra variable sets run as operations of their own, they don't count as generated
  operations = operations.flatMap((operation) => [operation, ...variants(operation, Number(options.assignments))]);

  for (const operation of operations) {
    row.ran++;
    let outcome = 'passed';
    let document;
    // the generator's operations are checked here too: one GraphQL rejects says nothing about
    // execution, only whether the router rejects it as well
    let invalid;
    try {
      document = parse(operation.document);
      session.data.variant.preferredTypes = preferredTypes(document);
      invalid = validate(session.supergraph.schema, document)[0]?.message;
    } catch (error) {
      outcome = 'harness-error';
      row.problems[`harness: ${error.message}`.slice(0, 160)] = 1;
    }
    // Clean, every check. With faults, values under a failed non-null field are fetched and never
    // reach the client, that's no over-fetching, so `unused` doesn't count there.
    for (const mode of !document ? [] : faults && !invalid ? ['clean', 'faults'] : ['clean']) {
      session.data.variant.faults = mode === 'faults';
      let result;
      let problems;
      const logOffset = statSync(session.routerLog).size;
      try {
        result = await session.runCase({
          operationText: operation.document,
          document,
          operationName: operation.operationName,
          variables: operation.variables,
          salt: operation.seed,
        });
        const errors = result.response.errors ?? [];
        if (invalid) {
          // rejected the way an invalid operation should be, by validation. Failing on it later,
          // in planning say, is a failure too
          const codes = errors.map((error) => error.extensions?.code);
          const rejected = result.response.data == null && codes.length && codes.every((code) => validationCodes.has(code));
          outcome = rejected ? 'invalid-rejected' : 'failed';
          problems =
            result.response.data == null
              ? [`router failed on an invalid operation instead of rejecting it (${codes.join(', ')}): ${invalid}`]
              : [`router accepted an invalid operation: ${invalid}`];
        } else if (
          result.response.data == null &&
          errors.length &&
          errors.every((error) => error.extensions?.code === 'QUERY_PLAN_BUILD_FAILED')
        ) {
          // the router couldn't plan it, the generated planner tests leave those alone too
          outcome = 'planner-refused';
          problems = [`planner: ${lastPlannerError(session.routerLog, logOffset)}`];
          (row.refusals ??= {})[operation.seed] = problems[0];
        } else {
          problems = result.problems.filter((problem) => mode === 'clean' || !problem.endsWith('unused values'));
          if (problems.length) outcome = 'failed';
        }
      } catch (error) {
        outcome = 'harness-error';
        problems = [`harness: ${error.message}`];
        result ??= { response: null, expected: null, log: [] };
      }
      // what the router sent to the subgraphs, to compare routers on the same operations
      if (result?.log) {
        const stats = { requests: 0, bytes: 0, representations: 0, duplicates: 0, outcome };
        for (const request of result.log) {
          stats.requests += 1;
          stats.bytes += JSON.stringify({ query: request.query, variables: request.variables ?? {} }).length;
          const seen = new Set();
          for (const [name, list] of Object.entries(request.variables ?? {})) {
            if (name !== 'representations' && !name.startsWith('__batch_reps')) continue;
            for (const entity of list ?? []) {
              const key = JSON.stringify(entity);
              stats.representations += 1;
              if (seen.has(key)) stats.duplicates += 1;
              seen.add(key);
            }
          }
        }
        (row.stats ??= {})[`${operation.seed}-${mode}`] = stats;
      }
      if (outcome === 'passed' || outcome === 'invalid-rejected') continue;

      if (outcome === 'failed') {
        for (const problem of problems) {
          const kind = `${mode === 'faults' ? '(faults) ' : ''}${problem.replace(/^\d+\+? /, '')}`;
          row.problems[kind] = (row.problems[kind] ?? 0) + 1;
        }
      } else {
        row.problems[problems[0].slice(0, 160)] = (row.problems[problems[0].slice(0, 160)] ?? 0) + 1;
      }
      writeFileSync(
        join(fixtureOut, `seed-${operation.seed}${mode === 'faults' ? '-faults' : ''}.json`),
        JSON.stringify(
          {
            outcome,
            problems,
            operation: operation.document,
            variables: operation.variables,
            dataDifferences: result.analysis?.dataDifferences,
            errorDifferences: result.analysis?.errorDifferences,
            issueExamples: result.analysis?.issueExamples,
            unused: result.analysis?.unused,
            response: result.response,
            expected: result.expected,
            subgraphRequests: result.log,
          },
          null,
          1,
        ),
      );
      // one failing run of an operation is enough to look at
      break;
    }
    row.cases[operation.seed] = outcome;
    row.outcomes[outcome] = (row.outcomes[outcome] ?? 0) + 1;
  }

  await session.shutdown();

  finish(row);
}

writeFileSync(join(outDir, 'summary.json'), JSON.stringify({ faults, ops: Number(options.ops), seed: Number(options.seed), summary }, null, 2));
// known exclusions are counted apart from passes, and fixtures that didn't run apart from both
const totals = { requested: 0, generated: 0, shortfall: 0, ran: 0, passed: 0, invalid: 0, failed: 0, harnessErrors: 0, knownRefusals: 0, otherRefusals: 0, knownSkips: 0, otherSkips: 0 };
for (const row of summary) {
  totals.ran += row.ran;
  // only fixtures that got as far as generating, the rest are counted as not run
  totals.requested += row.requested ?? 0;
  totals.generated += row.generated ?? 0;
  totals.shortfall += row.shortfall ?? 0;
  totals.passed += row.outcomes.passed ?? 0;
  totals.invalid += row.outcomes['invalid-rejected'] ?? 0;
  totals.failed += row.outcomes.failed ?? 0;
  totals.harnessErrors += row.outcomes['harness-error'] ?? 0;
  totals.knownRefusals += row.knownRefusals;
  totals.otherRefusals += (row.outcomes['planner-refused'] ?? 0) - row.knownRefusals;
  if (row.skipped) totals[row.unexpected.includes('skipped') ? 'otherSkips' : 'knownSkips']++;
}
const unexpected = summary.filter((row) => row.unexpected.length);
const shortRows = summary.filter((row) => row.shortfall);
console.log(
  `\n${summary.length} fixtures, ${totals.knownSkips + totals.otherSkips} not run (${totals.knownSkips} known). ` +
    `${totals.ran} operations: ${totals.passed} passed, ${totals.failed} failed, ${totals.harnessErrors} harness errors, ` +
    `${totals.invalid} invalid (generated wrong, the router's validation rejected them), ` +
    `${totals.otherRefusals} refused unexpectedly, ${totals.knownRefusals} known exclusions (refused, not checked). ` +
    `Details: ${outDir}/<fixture>/seed-<n>.json`,
);
// apart from the outcomes: what ran passed or not, this is what never ran
console.log(
  `Generated ${totals.generated} of ${totals.requested} operations requested from the fixtures that ran, ${totals.shortfall} short` +
    (shortRows.length ? `: ${shortRows.map((row) => `${row.fixture} ${row.generated}/${row.requested}`).join(', ')}` : '') +
    '.',
);
if (unexpected.length) {
  console.log(`Not green, ${unexpected.length} fixtures with unexpected outcomes:`);
  for (const row of unexpected) console.log(`  ${row.fixture}: ${row.unexpected.join(', ')}`);
}
process.exit(unexpected.length ? 1 : 0);
