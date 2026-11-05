#!/usr/bin/env node
// Runs a WFG operation through the router against mocked subgraphs, for several sets of variables,
// and checks the response against a monolith over the same data. See README.md.

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { parseArgs } from 'node:util';

import {
  getOperationAST,
  isEnumType,
  isInputObjectType,
  isListType,
  isNonNullType,
  Kind,
  parse,
  typeFromAST,
  visit,
} from 'graphql';

import { hash } from './src/data.js';
import { startSession } from './src/session.js';
import { stringifyScalars } from './src/supergraph.js';

const here = import.meta.dirname;
const repo = resolve(here, '../..');

const { values: options } = parseArgs({
  options: {
    operation: { type: 'string', default: 'query-2' },
    supergraph: { type: 'string', default: join(repo, 'bin/router/fixture/wfg/supergraph.graphql') },
    router: { type: 'string', default: join(repo, 'target/release/hive_router') },
    // quick: all true, all false and some random ones. full: also every condition flipped alone.
    cases: { type: 'string', default: 'quick' },
    random: { type: 'string', default: '6' },
    'subgraph-port': { type: 'string', default: '4210' },
    'router-port': { type: 'string', default: '4211' },
    out: { type: 'string', default: join(here, 'out') },
    // keep `Int`, `Float` and `Boolean` fields as they are, by default they return `String`s
    'keep-scalars': { type: 'boolean', default: false },
    // the response and every subgraph request of a case: failed (only failing cases), all, none
    logs: { type: 'string', default: 'failed' },
  },
});

const operationPath = options.operation.endsWith('.graphql')
  ? resolve(options.operation)
  : join(repo, 'bin/router/fixture/wfg', `${options.operation}.graphql`);
const outDir = resolve(options.out);
mkdirSync(outDir, { recursive: true });

let sdl = readFileSync(options.supergraph, 'utf8');
let stringified = 0;
if (!options['keep-scalars']) ({ sdl, changed: stringified } = stringifyScalars(sdl));

const session = await startSession({
  sdl,
  router: options.router,
  subgraphPort: Number(options['subgraph-port']),
  routerPort: Number(options['router-port']),
  outDir,
});
const { supergraph, data } = session;
process.on('SIGINT', () => {
  session.shutdown();
  process.exit(130);
});

const operationText = readFileSync(operationPath, 'utf8');
const document = parse(operationText);
const operation = getOperationAST(document);
const operationName = operation.name?.value;

// --- variables

function generate(type, depth) {
  if (isNonNullType(type)) return generate(type.ofType, depth);
  if (isListType(type)) return [generate(type.ofType, depth)];
  if (isInputObjectType(type)) {
    const value = {};
    for (const field of Object.values(type.getFields())) {
      // required fields always, optional ones only at the top, recursive inputs stop that way
      if (isNonNullType(field.type) || depth === 0) value[field.name] = generate(field.type, depth + 1);
    }
    return value;
  }
  if (isEnumType(type)) return type.getValues()[0].name;
  switch (type.name) {
    case 'Int':
      return 1;
    case 'Float':
      return 1.5;
    case 'Boolean':
      return true;
    case 'ID':
      return 'id-1';
    default:
      return 's';
  }
}

const conditionVariables = new Set();
visit(document, {
  InlineFragment(node) {
    if (node.typeCondition) data.variant.preferredTypes.add(node.typeCondition.name.value);
  },
  FragmentDefinition(node) {
    data.variant.preferredTypes.add(node.typeCondition.name.value);
  },
  Directive(node) {
    if (node.name.value !== 'include' && node.name.value !== 'skip') return;
    const condition = node.arguments?.find((arg) => arg.name.value === 'if')?.value;
    if (condition?.kind === Kind.VARIABLE) conditionVariables.add(condition.name.value);
  },
});
const conditions = [...conditionVariables].sort();

const baseVariables = {};
for (const definition of operation.variableDefinitions ?? []) {
  baseVariables[definition.variable.name.value] = generate(typeFromAST(supergraph.schema, definition.type), 0);
}

const withConditions = (values) => ({
  ...baseVariables,
  ...Object.fromEntries(conditions.map((name, index) => [name, values[index]])),
});

const cases = [
  { name: 'all-true', variables: withConditions(conditions.map(() => true)) },
  { name: 'all-false', variables: withConditions(conditions.map(() => false)) },
];
for (let round = 0; round < Number(options.random); round++) {
  cases.push({
    name: `random-${round}`,
    variables: withConditions(conditions.map((name) => hash(`${round}|${name}`) % 2 === 0)),
  });
}
if (options.cases === 'full') {
  conditions.forEach((name, index) => {
    for (const base of [true, false]) {
      const values = conditions.map(() => base);
      values[index] = !base;
      cases.push({ name: `only-${name}-${!base}`, variables: withConditions(values) });
    }
  });
}

// --- cases

console.log(
  `${options.operation}: ${conditions.length} condition variables, ${cases.length} cases, ` +
    `${supergraph.graphNames.length} subgraphs, ${stringified} Int/Float/Boolean fields as String`,
);

const results = [];
let failed = false;
for (const [index, testCase] of cases.entries()) {
  const { response, expected, analysis, problems, took, log } = await session.runCase({
    operationText,
    document,
    operationName,
    variables: testCase.variables,
    salt: index,
  });
  if (problems.length) failed = true;

  console.log(
    [
      testCase.name.padEnd(28),
      `${String(took).padStart(6)}ms`,
      `${analysis.requests} requests`,
      `${analysis.lookups} lookups (${analysis.repeatedLookups} repeated)`,
      `unused: ${analysis.unused.fields} fields, ${analysis.unused.keys} keys`,
      problems.length ? `FAIL ${problems.join(', ')}` : 'ok',
    ].join(' | '),
  );

  results.push({ ...testCase, took, ...analysis });
  if (options.logs === 'all' || (options.logs === 'failed' && problems.length)) {
    writeFileSync(
      join(outDir, `${testCase.name}.log.json`),
      JSON.stringify({ response, expected, subgraphRequests: log }, null, 1),
    );
  }
}

writeFileSync(join(outDir, 'report.json'), JSON.stringify({ operation: options.operation, results }, null, 2));
console.log(`report: ${join(outDir, 'report.json')}`);
session.shutdown();
process.exit(failed ? 1 : 0);
