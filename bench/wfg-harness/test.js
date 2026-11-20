// Quick checks of the harness itself that don't need a router: `node --test test.js`

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';

import { switchedOffInputs } from './src/analyze.js';
import { loadSupergraph } from './src/supergraph.js';

const fixture = (name) =>
  loadSupergraph(readFileSync(join(import.meta.dirname, '../../bin/router/fixture/tests', name), 'utf8'));

test('a key of another subgraph is not enough', () => {
  const { hasKeyIn } = fixture('complex-entity-call.supergraph.graphql');
  // `price` resolves Product by `id pid category { id tag }`, `products` by `id`
  assert.equal(hasKeyIn('Product', 'price', { __typename: 'Product', id: 'Product:1' }), false);
  assert.equal(
    hasKeyIn('Product', 'price', { __typename: 'Product', id: 'Product:1', pid: 'p', category: { id: 'c' } }),
    false,
    'a nested key field is missing',
  );
  assert.equal(
    hasKeyIn('Product', 'price', {
      __typename: 'Product',
      id: 'Product:1',
      pid: 'p',
      category: { id: 'c', tag: 't' },
    }),
    true,
  );
  assert.equal(hasKeyIn('Product', 'products', { __typename: 'Product', id: 'Product:1' }), true);
});

test('a malformed key value is not a key, and does not throw', () => {
  const { hasKeyIn } = fixture('complex-entity-call.supergraph.graphql');
  const product = { __typename: 'Product', id: 'Product:1', pid: 'p' };
  // `category { id tag }` needs an object
  assert.equal(hasKeyIn('Product', 'price', { ...product, category: 'broken' }), false);
  assert.equal(hasKeyIn('Product', 'price', { ...product, category: 7 }), false);
  // `products { id pid }` needs objects in its list
  assert.equal(hasKeyIn('ProductList', 'list', { __typename: 'ProductList', products: ['broken'] }), false);
  assert.equal(
    hasKeyIn('ProductList', 'list', { __typename: 'ProductList', products: [{ id: '1', pid: 'p' }, null] }),
    true,
  );
  // a null key value is still for the data checks to judge
  assert.equal(hasKeyIn('Product', 'price', { ...product, category: null }), true);
});

test('both batch alias formats are read', () => {
  const { schema } = fixture('complex-entity-call.supergraph.graphql');
  const requires = [{ kind: 'InlineFragment', typeCondition: 'Product', selections: [{ kind: 'Field', name: 'id' }] }];
  const batch = (alias) => ({ node: { kind: 'BatchFetch', entityBatch: { aliases: [{ requires, ...alias }] } } });

  // before `pathGroups`, an alias has plain `paths` and always runs
  assert.deepEqual([...switchedOffInputs(batch({ paths: [] }), {}, schema)], []);
  assert.deepEqual([...switchedOffInputs(batch({ pathGroups: [{ paths: [] }] }), {}, schema)], []);
  assert.deepEqual(
    [...switchedOffInputs(batch({ pathGroups: [{ runIf: [{ includeIf: ['a'] }], paths: [] }] }), { a: false }, schema)],
    ['Product (key)', 'Product.id'],
  );
});
