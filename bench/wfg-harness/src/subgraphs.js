// Every subgraph of the supergraph on one HTTP server, at `/<subgraph name>`. Each request and what
// it got back goes to `log`.

import { createServer } from 'node:http';

import { parse } from 'graphql';

export function startSubgraphs({ executor, graphNames, port }) {
  const log = [];
  // the self-test breaks requests and results on purpose: `request(graph, body)` before a
  // subgraph runs one, `result(graph, body, result)` before it answers
  const hooks = {};
  const known = new Set(graphNames);
  const documents = new Map();

  const server = createServer(async (request, response) => {
    const graph = decodeURIComponent(new URL(request.url, 'http://localhost').pathname.slice(1));
    if (request.method !== 'POST' || !known.has(graph)) {
      response.writeHead(404).end();
      return;
    }
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    let body;
    let document;
    try {
      body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      document = documents.get(body.query);
      if (!document) {
        document = parse(body.query);
        documents.set(body.query, document);
      }
    } catch (error) {
      log.push({ graph, issues: [{ kind: 'bad-request', detail: String(error) }], lookups: [] });
      response.writeHead(400, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ errors: [{ message: String(error) }] }));
      return;
    }

    hooks.request?.(graph, body);
    const { result, issues, lookups, localInputs } = await executor.run({
      document,
      variables: body.variables,
      operationName: body.operationName,
      graph,
    });
    hooks.result?.(graph, body, result);
    log.push({ graph, query: body.query, variables: body.variables, result, issues, lookups, localInputs });
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify(result));
  });

  return new Promise((resolve) => {
    server.listen(port, '127.0.0.1', () => resolve({ server, log, hooks, url: `http://127.0.0.1:${port}` }));
  });
}
