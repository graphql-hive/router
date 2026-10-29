// Generated operations against a fixture supergraph, shared by fuzz.js and selftest.js.

import { spawnSync } from 'node:child_process';

import { visit } from 'graphql';

import { stringifyScalars } from './supergraph.js';

/** A fixture's directory in a run's output, from its path under `bin/router/fixture`. */
export const outName = (fixture) => fixture.replace(/\.graphql$/, '').replaceAll('/', '-');

/** What the harness can't model is left out of the supergraph both sides get. */
export function prepare(sdl) {
  // the router would hide `@authenticated` fields from an anonymous request, the monolith wouldn't
  const withoutAuth = sdl.replace(/(?<!directive )@(authenticated|requiresScopes\([^)]*\)|policy\([^)]*\))/g, '');
  return stringifyScalars(withoutAuth).sdl;
}

/** `count` operations from `qp-dev-cli generate`, seeds from `first` on. */
export function generate(devCli, supergraphPath, count, first) {
  const result = spawnSync(devCli, ['generate', supergraphPath, String(count), String(first)], {
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.split('\n').slice(-5).join('\n'));
  return result.stdout
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

/** The types the operation's fragments name, abstract values take those when they can. */
export function preferredTypes(document) {
  const types = new Set();
  visit(document, {
    InlineFragment(node) {
      if (node.typeCondition) types.add(node.typeCondition.name.value);
    },
    FragmentDefinition(node) {
      types.add(node.typeCondition.name.value);
    },
  });
  return types;
}
