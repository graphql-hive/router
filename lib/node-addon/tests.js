import fs from "node:fs/promises";
import path from "node:path";
import { describe, it } from "node:test";
import { QueryPlanner } from "./index.js";

describe("fixtures", async () => {
  for (const fixtureName of await fs.readdir("fixture")) {
    const fixtureDir = path.join("fixture", fixtureName);
    const supergraph = await fs.readFile(
      path.join(fixtureDir, "supergraph.graphql"),
      "utf-8"
    );
    for (const queryFile of await fs.readdir(fixtureDir)) {
      if (queryFile === "supergraph.graphql") continue;
      if (!queryFile.endsWith(".graphql")) continue;
      const query = await fs.readFile(
        path.join(fixtureDir, queryFile),
        "utf-8"
      );
      it(`should plan ${fixtureName}/${queryFile}`, async (t) => {
        const planner = new QueryPlanner(supergraph);
        const plan = await planner.plan(query);
        t.assert.snapshot(plan);
      });
    }
  }
});

it("should expose consumer schema without federation internals", async (t) => {
  const supergraph = await fs.readFile(
    path.join("fixture", "simple-inaccessible", "supergraph.graphql"),
    "utf-8"
  );
  const planner = new QueryPlanner(supergraph);
  t.assert.snapshot(planner.consumerSchema);
});
