// Runs one operation against a router and prints the subgraph requests, the problems and the
// router's response. The operation file is a fuzz `seed-<n>.json`, a `reduce.mjs` output, or
// `{ "document", "variables", "seed" }`.
// node run-manual.mjs <router> <port> <supergraph> <operation-file> <out>
import { readFileSync } from 'node:fs';
import { parse } from 'graphql';
import { startSession } from './src/session.js';
import { preferredTypes } from './src/generated.js';

const [router, port, sdlPath, opFile, out] = process.argv.slice(2);
const op = JSON.parse(readFileSync(opFile, 'utf8'));
const operationText = op.document ?? op.operation;
const seed = op.seed ?? Number(opFile.match(/seed-(\d+)/)?.[1] ?? 0);
const s = await startSession({ sdl: readFileSync(sdlPath, 'utf8'), router, subgraphPort: +port, routerPort: +port + 1, outDir: out });
const document = parse(operationText);
s.data.variant.preferredTypes = preferredTypes(document);
const r = await s.runCase({ operationText, document, operationName: op.operationName, variables: op.variables, salt: seed });
for (const req of r.log) console.log(req.graph, req.query, JSON.stringify(req.variables));
console.log('problems', JSON.stringify(r.problems));
console.log('router', JSON.stringify(r.response).slice(0, 600));
await s.shutdown();
process.exit(0);
