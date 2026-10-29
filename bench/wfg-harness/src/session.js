// One supergraph: its subgraphs on one HTTP server, the router in front of them, the monolith
// next to it. `runCase` sends an operation through the router and checks it.

import { spawn } from 'node:child_process';
import { mkdirSync, openSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { analyzeCase, problemsOf, switchedOffInputs, withDefaults } from './analyze.js';
import { createData } from './data.js';
import { createExecutor } from './execute.js';
import { startSubgraphs } from './subgraphs.js';
import { loadSupergraph, withSubgraphUrls } from './supergraph.js';

// routers still running, a crash in a runner doesn't leave one behind on its port
const routers = new Set();
globalThis.process.once('exit', () => {
  for (const router of routers) router.kill();
});

export async function startSession({ sdl, router, subgraphPort, routerPort, outDir }) {
  mkdirSync(outDir, { recursive: true });
  const supergraph = loadSupergraph(sdl);
  const data = createData(supergraph);
  const executor = createExecutor(supergraph, data);

  const subgraphs = await startSubgraphs({ executor, graphNames: supergraph.graphNames, port: subgraphPort });

  const supergraphPath = join(outDir, 'supergraph.graphql');
  writeFileSync(supergraphPath, withSubgraphUrls(sdl, subgraphs.url));
  const configPath = join(outDir, 'router.yaml');
  writeFileSync(
    configPath,
    `supergraph:
  source: file
  path: ${supergraphPath}
http:
  host: 127.0.0.1
  port: ${routerPort}
query_planner:
  timeout: 600s
  allow_expose: true
traffic_shaping:
  router:
    request_timeout: 600s
  all:
    request_timeout: 600s
limits:
  max_request_body_size: 20MB
error_masking:
  all:
    enabled: false
log:
  level: warn
`,
  );

  const routerLog = join(outDir, 'router.log');
  const logFile = openSync(routerLog, 'w');
  const process = spawn(router, [], {
    env: { ...globalThis.process.env, ROUTER_CONFIG_FILE_PATH: configPath },
    stdio: ['ignore', logFile, logFile],
  });
  let exited = false;
  let spawnError;
  routers.add(process);
  process.on('exit', () => {
    exited = true;
    routers.delete(process);
  });
  // a missing binary, say. Without a listener it would take the harness down
  process.on('error', (error) => {
    exited = true;
    spawnError = error;
  });

  /** Stops the router and the subgraphs, resolves once both are gone. */
  function shutdown() {
    const gone = exited ? Promise.resolve() : new Promise((done) => process.once('exit', done));
    process.kill();
    subgraphs.server.closeAllConnections?.();
    return Promise.all([gone, new Promise((done) => subgraphs.server.close(done))]);
  }

  const routerUrl = `http://127.0.0.1:${routerPort}`;
  for (let attempt = 0; ; attempt++) {
    if (exited) {
      subgraphs.server.close();
      throw new Error(spawnError ? `router didn't run: ${spawnError.message}` : `router exited, see ${routerLog}`);
    }
    try {
      if ((await fetch(`${routerUrl}/health`)).ok) break;
    } catch {}
    if (attempt > 600) {
      shutdown();
      throw new Error('router not healthy after 60s');
    }
    await new Promise((done) => setTimeout(done, 100));
  }

  /**
   * Sends `operationText` with `variables` to the router, runs it on the monolith, and compares.
   * `salt` picks which types single abstract values take.
   */
  async function runCase({ operationText, document, operationName, variables, salt = 0, mutateResponse }) {
    data.variant.salt = salt;
    subgraphs.log.length = 0;
    const started = performance.now();
    const response = await fetch(`${routerUrl}/graphql`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', accept: 'application/json', 'hive-expose-query-plan': 'true' },
      body: JSON.stringify({ query: operationText, variables, operationName }),
    }).then((res) => res.json());
    // the self-test breaks the response on purpose
    mutateResponse?.(response);
    const took = Math.round(performance.now() - started);
    const expected = (await executor.run({ document, variables, operationName })).result;
    const switchedOff = switchedOffInputs(response.extensions?.queryPlan, withDefaults(document, operationName, variables), supergraph.schema);
    const analysis = analyzeCase({ response, expected, log: subgraphs.log, switchedOff, inputField: supergraph.inputField,
      requiresUnder: supergraph.requiresUnder,
    });
    return { response, expected, analysis, problems: problemsOf(analysis), took, log: [...subgraphs.log] };
  }

  return { supergraph, data, executor, runCase, shutdown, routerLog, subgraphHooks: subgraphs.hooks };
}
