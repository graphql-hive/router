import fs from "node:fs/promises";
import path from "node:path";
import { describe, it } from "node:test";
import { QueryPlanner } from "../dist/index.js";

const PLAN_METHODS = ['plan', 'planAsync'] as const;

describe("fixtures", async () => {
  for (const fixtureName of await fs.readdir("fixture")) {
    const fixtureDir = path.join("fixture", fixtureName);
    const supergraph = await fs.readFile(
      path.join(fixtureDir, "supergraph.graphql"),
      "utf-8",
    );
    for (const queryFile of await fs.readdir(fixtureDir)) {
      if (queryFile === "supergraph.graphql") continue;
      if (!queryFile.endsWith(".graphql")) continue;
      const query = await fs.readFile(
        path.join(fixtureDir, queryFile),
        "utf-8",
      );
      for (const planMethod of PLAN_METHODS) {
        it(`should ${planMethod} ${fixtureName}/${queryFile}`, async (t) => {
          const planner = new QueryPlanner(supergraph);
          const plan = await planner[planMethod](query, undefined, new Set(), 0);
          t.assert.snapshot(plan);
        });
      }
    }
  }
});

it("should expose consumer schema without federation internals", async (t) => {
  const supergraph = await fs.readFile(
    path.join("fixture", "simple-inaccessible", "supergraph.graphql"),
    "utf-8",
  );
  const planner = new QueryPlanner(supergraph);
  t.assert.snapshot(planner.consumerSchema);
});

function collectFetches(node: any, fetches: any[] = []): any[] {
  if (!node || typeof node !== "object") return fetches;
  if (node.kind === "Fetch" || node.kind === "BatchFetch") fetches.push(node);
  for (const value of Object.values(node)) collectFetches(value, fetches);
  return fetches;
}

it("should expose fetch ids and dependencies", async (t) => {
  const supergraph = await fs.readFile(
    path.join("fixture", "federation-example", "supergraph.graphql"),
    "utf-8",
  );
  const planner = new QueryPlanner(supergraph);

  const plan = planner.plan(
    "{ topProducts { upc inStock reviews { author { name } } } }",
    undefined,
    new Set(),
    0,
  );
  const fetches = Object.fromEntries(
    collectFetches(plan.node).map((fetch) => [fetch.serviceName, fetch]),
  );
  const { products, inventory, reviews, accounts } = fetches;
  t.assert.deepStrictEqual(products.dependsOn, []);
  t.assert.deepStrictEqual(inventory.dependsOn, [products.id]);
  t.assert.deepStrictEqual(reviews.dependsOn, [products.id]);
  // authors only need the reviews, not the unrelated inventory sibling
  t.assert.deepStrictEqual(accounts.dependsOn, [reviews.id]);

  const query = await fs.readFile(
    path.join("fixture", "federation-example", "query.graphql"),
    "utf-8",
  );
  const batches = collectFetches(
    planner.plan(query, undefined, new Set(), 0).node,
  ).filter((fetch) => fetch.kind === "BatchFetch");
  t.assert.ok(batches.length > 0);
  for (const batch of batches) {
    t.assert.strictEqual(batch.completes[0], batch.id);
    t.assert.ok(batch.completes.length > 1);
  }
});
