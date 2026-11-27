// Shrinks a failing fuzz case: drops selections and directives while the same kind of problem
// still shows. node reduce.mjs <router> <port> <supergraph> <seed-file> <kind> <out>
import { readFileSync, writeFileSync } from 'node:fs';
import { parse, print, visit, validate, BREAK } from 'graphql';
import { startSession } from './src/session.js';
import { preferredTypes } from './src/generated.js';

const [router, port, sdlPath, seedFile, kind, out] = process.argv.slice(2);
const seed = JSON.parse(readFileSync(seedFile, 'utf8'));
const salt = Number(seedFile.match(/seed-(\d+)/)[1]);
const s = await startSession({ sdl: readFileSync(sdlPath, 'utf8'), router, subgraphPort: +port, routerPort: +port + 1, outDir: out });

// drop variable definitions and fragments nothing uses any more
function tidy(document) {
  for (let i = 0; i < 5; i++) {
    const used = new Set();
    const spreads = new Set();
    visit(document, {
      Variable(node, key, parent) { if (parent?.kind !== 'VariableDefinition') used.add(node.name.value); },
      FragmentSpread(node) { spreads.add(node.name.value); },
    });
    document = visit(document, {
      VariableDefinition(node) { return used.has(node.variable.name.value) ? undefined : null; },
      FragmentDefinition(node) { return spreads.has(node.name.value) ? undefined : null; },
    });
  }
  return document;
}

async function fails(document) {
  // printing an empty selection set gives text that doesn't parse
  try { document = parse(print(document)); } catch { return false; }
  if (validate(s.supergraph.schema, document).length) return false;
  const text = print(document);
  s.data.variant.preferredTypes = preferredTypes(document);
  try {
    const r = await s.runCase({ operationText: text, document, operationName: undefined, variables: seed.variables, salt });
    return r.problems.some((p) => p.includes(kind));
  } catch { return false; }
}

// every node we could take out, by a running index
function removable(document) {
  let count = 0;
  visit(document, { Field() { count++; }, InlineFragment() { count++; }, FragmentSpread() { count++; }, Directive() { count++; } });
  return count;
}
function without(document, target) {
  let i = 0;
  const strip = () => (i++ === target ? null : undefined);
  const result = visit(document, { Field: strip, InlineFragment: strip, FragmentSpread: strip, Directive: strip });
  // a selection set left empty is invalid, validation throws it out
  return tidy(result);
}

let document = tidy(parse(seed.operation));
if (!(await fails(document))) { console.log('does not reproduce'); await s.shutdown(); process.exit(1); }
let changed = true;
while (changed) {
  changed = false;
  for (let target = removable(document) - 1; target >= 0; target--) {
    const candidate = without(document, target);
    if (await fails(candidate)) { document = candidate; changed = true; }
  }
}
const text = print(document);
writeFileSync(`${out}/reduced.graphql`, text);
const final = await s.runCase({ operationText: text, document, variables: seed.variables, salt });
writeFileSync(`${out}/reduced.json`, JSON.stringify({ operation: text, variables: seed.variables, problems: final.problems, response: final.response, expected: final.expected, requests: final.log }, null, 2));
console.log(text);
console.log(JSON.stringify(seed.variables));
console.log(final.problems);
await s.shutdown();
process.exit(0);
