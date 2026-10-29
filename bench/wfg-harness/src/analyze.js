// What one case tells: does the router's data match the monolith's, and what did it ask the
// subgraphs for that it never used.

import { getOperationAST, isAbstractType, valueFromASTUntyped } from 'graphql';

import { INJECTED } from './execute.js';

/**
 * Paths where `actual` differs from `expected`, at most `limit`. `nullsOk(path)` says where a null
 * the router gave instead of a value isn't a difference.
 */
export function diffData(expected, actual, limit = 20, nullsOk = () => false) {
  const differences = [];
  const walk = (left, right, path) => {
    if (differences.length >= limit) return;
    if (right === null && left !== null && nullsOk(path)) return;
    if (Array.isArray(left) || Array.isArray(right)) {
      if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) {
        differences.push({ path, expected: left, actual: right });
        return;
      }
      left.forEach((item, index) => walk(item, right[index], `${path}.${index}`));
      return;
    }
    if (left && right && typeof left === 'object' && typeof right === 'object') {
      for (const key of new Set([...Object.keys(left), ...Object.keys(right)])) {
        if (!(key in right)) differences.push({ path: `${path}.${key}`, missing: true, expected: left[key] });
        else if (!(key in left)) differences.push({ path: `${path}.${key}`, unexpected: true, actual: right[key] });
        else walk(left[key], right[key], `${path}.${key}`);
        if (differences.length >= limit) return;
      }
      return;
    }
    if (left !== right) differences.push({ path, expected: left, actual: right });
  };
  walk(expected, actual, 'data');
  return differences;
}

function strings(value, out = new Set()) {
  if (typeof value === 'string') out.add(value);
  else if (Array.isArray(value)) for (const item of value) strings(item, out);
  else if (value && typeof value === 'object') {
    for (const [key, inner] of Object.entries(value)) if (key !== '__typename') strings(inner, out);
  }
  return out;
}

// `Parent.field:hash` for a plain value, `Type:seed` for a key
const PLAIN = /^([A-Za-z0-9_]+)\.([A-Za-z0-9_]+):[0-9a-z]+$/;
const KEY = /^([A-Za-z0-9_]+):\d+$/;

/**
 * Every `String`/`ID` the subgraphs returned is unique, so one that's neither in the client's
 * response nor sent to another subgraph later (a key, a `@requires` field) was fetched for nothing.
 */
export function unusedValues(clientData, log, switchedOff = new Set()) {
  const used = strings(clientData);
  for (const entry of log) {
    strings(entry.variables, used);
    strings(entry.localInputs, used);
  }

  const byGraph = {};
  const byCoordinate = {};
  let keys = 0;
  let fields = 0;
  for (const entry of log) {
    for (const value of strings(entry.result?.data)) {
      if (used.has(value)) continue;
      const plain = PLAIN.exec(value);
      const key = !plain && KEY.exec(value);
      if (!plain && !key) continue;
      const coordinate = plain ? `${plain[1]}.${plain[2]}` : `${key[1]} (key)`;
      if (switchedOff.has(coordinate)) continue;
      if (plain) fields++;
      else keys++;
      byGraph[entry.graph] = (byGraph[entry.graph] ?? 0) + 1;
      byCoordinate[coordinate] = (byCoordinate[coordinate] ?? 0) + 1;
    }
  }
  return { fields, keys, byGraph, byCoordinate };
}

/** The variables with the operation's defaults filled in. */
export function withDefaults(document, operationName, variables = {}) {
  const values = {};
  for (const definition of getOperationAST(document, operationName)?.variableDefinitions ?? []) {
    if (definition.defaultValue) values[definition.variable.name.value] = valueFromASTUntyped(definition.defaultValue);
  }
  return { ...values, ...variables };
}

/**
 * Keys and `@requires` inputs of the entity calls the variables switch off, as `Type (key)` and
 * `Type.field`. The parent fetch selects them anyway, only the call is
 * skipped.
 */
export function switchedOffInputs(plan, variables, schema) {
  const out = new Set();
  if (!plan) return out;
  const types = (name) => {
    const type = schema.getType(name);
    return type && isAbstractType(type) ? schema.getPossibleTypes(type).map((each) => each.name) : [name];
  };
  const add = (selections, type) => {
    for (const selection of selections ?? []) {
      if (selection.kind === 'InlineFragment') add(selection.selections, selection.typeCondition ?? type);
      else if (type && selection.name !== '__typename') {
        for (const name of types(type)) out.add(`${name} (key)`).add(`${name}.${selection.name}`);
      }
    }
  };
  const holds = (group) =>
    (group.includeIf ?? []).every((name) => variables[name] === true) &&
    (group.skipIf ?? []).every((name) => variables[name] !== true);
  const walk = (node, off) => {
    if (!node) return;
    switch (node.kind) {
      case 'Fetch':
        if (off || (node.runIf && !node.runIf.some(holds))) add(node.requires);
        break;
      case 'BatchFetch':
        // a group that's off leaves its paths out, the alias may still run for the others
        for (const alias of node.entityBatch.aliases) {
          if (off || alias.pathGroups.some((group) => group.runIf && !group.runIf.some(holds))) add(alias.requires);
        }
        break;
      case 'Condition':
        walk(node.ifClause, off || variables[node.condition] !== true);
        walk(node.elseClause, off || variables[node.condition] === true);
        break;
      case 'Flatten':
        walk(node.node, off);
        break;
      default:
        for (const child of node.nodes ?? []) walk(child, off);
    }
  };
  walk(plan.node, false);
  return out;
}

/**
 * Whether the router's errors make sense. How many errors a failure gives isn't fixed: when a
 * non-null field fails, graphql-js reports the one it hit first, the router every failing field.
 * What has to hold: an error points at a null, or below one, never at data the client got, and
 * when the monolith has errors the router has some too. A wrong null is a data difference already.
 * An error the router gives for a `@requires` or context input that failed points at the object
 * that needed it, but those cases are hidden faults, and then nothing here is checked.
 */
export function errorDifferences(expected, actual, actualData, limit = 10, inputField = () => false) {
  // whether the client got a value at `path`, an error there would be about data it has
  const hasValueAt = (path) => {
    let value = actualData;
    for (const step of path) {
      if (value == null) return false;
      value = value[step];
    }
    return value != null;
  };
  // a field the router fetched as a `@requires` or context input failed, the router puts that
  // error on the object that needed it. The client can have asked for the field elsewhere too.
  // The object has to say it's the failed field's type, by `__typename` or by its values
  const aboutAnInput = (error) => {
    if (!INJECTED.test(error.message)) return false;
    const [typeName, fieldName] = error.message.split(' ')[0].split('.');
    const value = error.path.reduce((object, step) => object?.[step], actualData);
    return value && typeof value === 'object' && inputField(fieldName) && typeOf(value) === typeName;
  };
  return {
    // the monolith has errors, the router none
    missing: (expected ?? []).length && !(actual ?? []).length ? [(expected ?? [])[0].message] : [],
    // the router has an error on a value or an object the client got
    unexpected: (actual ?? [])
      .filter((error) => error.path && hasValueAt(error.path) && !aboutAnInput(error))
      .slice(0, limit)
      .map((error) => `${JSON.stringify(error.path)}: ${error.message}`),
    // a router error with no path is about the request, not a field
    withoutPath: (actual ?? []).filter((error) => !error.path).slice(0, limit).map((error) => error.message),
  };
}

/**
 * What type an object of the client's data is: its `__typename`, or what its own values say, a
 * `Type.field:hash` value or a `Type:seed` key. `undefined` when nothing does.
 */
function typeOf(object) {
  if (object.__typename) return object.__typename;
  for (const value of Object.values(object)) {
    if (typeof value !== 'string') continue;
    const match = PLAIN.exec(value) ?? KEY.exec(value);
    if (match) return match[1];
  }
  return undefined;
}

/** Whether a null at a `diffData` path has a router error about a hidden fault at, above or below it. */
function explainedByErrors(errors = [], hidden) {
  const paths = errors
    .filter((error) => error.path && hidden.has(error.message))
    .map((error) => ['data', ...error.path].join('.'));
  const related = (a, b) => a === b || a.startsWith(`${b}.`) || b.startsWith(`${a}.`);
  return (path) => paths.some((errorPath) => related(path, errorPath));
}

/** Without the router's errors about hidden faults on data the client got, the monolith can't say where those go. */
const withoutHidden = (differences, hidden) => ({
  ...differences,
  unexpected: differences.unexpected.filter((line) => !hidden.has(line.slice(line.indexOf(']: ') + 3))),
});

/** Everything that fails a case, as short lines. */
export function problemsOf(analysis) {
  const problems = [];
  if (analysis.dataDifferences.length) problems.push(`${analysis.dataDifferences.length}+ data differences`);
  const { missing = [], unexpected = [], withoutPath = [] } = analysis.errorDifferences ?? {};
  if (missing.length) problems.push(`${missing.length} errors missing`);
  if (unexpected.length) problems.push(`${unexpected.length} unexpected errors`);
  if (withoutPath.length) problems.push(`${withoutPath.length} request errors`);
  for (const [kind, count] of Object.entries(analysis.issues)) problems.push(`${count} ${kind}`);
  const unused = analysis.unused.fields + analysis.unused.keys;
  if (unused) problems.push(`${unused} unused values`);
  return problems;
}

export function analyzeCase({ response, expected, log, switchedOff, inputField, requiresUnder = () => false }) {
  const requestsByGraph = {};
  const issues = {};
  const issueExamples = {};
  const lookupsByGraphEntity = new Map();
  let lookups = 0;
  // A `@requires` or context input that failed in a subgraph but never in the monolith wasn't
  // asked by the client, the router fetched it for itself. The monolith doesn't resolve those, so
  // its result can't tell what the router should send back then. Any other field the monolith
  // didn't report, it never got to: graphql-js stops at the first failing non-null field
  const failedInMonolith = new Set((expected.errors ?? []).map((error) => error.message));
  const hiddenFaults = [
    ...new Set(
      log.flatMap((entry) =>
        (entry.result?.errors ?? [])
          .map((error) => error.message)
          .filter(
            (message) =>
              INJECTED.test(message) && !failedInMonolith.has(message) && inputField(message.split(' ')[0].split('.')[1]),
          ),
      ),
    ),
  ];
  const hidden = new Set(hiddenFaults);
  const hiddenFields = hiddenFaults.map((message) => message.split(' ')[0].split('.')[1]);
  // it didn't decode because of nulls in its key, and the failed input explains every one of them:
  // a `@requires` of the entity asks for the failed field there or below. A key field missing or
  // holding something that isn't a value of this data has another cause
  const causedByHiddenFault = (causes) =>
    hiddenFields.length > 0 &&
    causes != null &&
    !causes.missing.length &&
    !causes.malformed.length &&
    causes.nulls.length > 0 &&
    causes.nulls.every((path) => hiddenFields.some((field) => requiresUnder(causes.typeName, path, field)));

  for (const entry of log) {
    requestsByGraph[entry.graph] = (requestsByGraph[entry.graph] ?? 0) + 1;
    for (const issue of entry.issues) {
      // the router sends `null` for an object a failed input was in, a key one too, then
      // the subgraph can't find the entity
      if (issue.kind === 'undecodable-representation' && causedByHiddenFault(issue.causes)) continue;
      issues[issue.kind] = (issues[issue.kind] ?? 0) + 1;
      (issueExamples[issue.kind] ??= new Set()).add(`${entry.graph}: ${issue.detail}`);
    }
    for (const lookup of entry.lookups) {
      lookups++;
      const key = `${entry.graph}/${lookup.entity}`;
      lookupsByGraphEntity.set(key, (lookupsByGraphEntity.get(key) ?? 0) + 1);
    }
  }
  const repeatedLookups = [...lookupsByGraphEntity.values()].filter((count) => count > 1).length;

  return {
    hiddenFaults,
    // a fault the monolith never sees can only make the router's data null where a field needed
    // what failed, and put an error on the object that needed it. The router says so with an error
    // there, so a null counts only where one of its errors is at, above or below it. Everything
    // else still counts
    dataDifferences: diffData(
      expected.data,
      response.data,
      20,
      hiddenFaults.length > 0 ? explainedByErrors(response.errors, hidden) : undefined,
    ),
    errorDifferences: withoutHidden(
      errorDifferences(expected.errors, response.errors, response.data, 10, inputField),
      hidden,
    ),
    requests: log.length,
    requestsByGraph,
    lookups,
    repeatedLookups,
    issues,
    issueExamples: Object.fromEntries(
      Object.entries(issueExamples).map(([kind, examples]) => [kind, [...examples].slice(0, 10)]),
    ),
    unused: unusedValues(response.data, log, switchedOff),
  };
}
