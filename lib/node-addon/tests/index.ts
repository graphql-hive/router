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

it("should limit context groups per request with the alias limits", async (t) => {
  const supergraph = (fixture: string) =>
    fs.readFile(`../../bin/router/fixture/tests/${fixture}.supergraph.graphql`, "utf-8");
  const contextual = (planner: QueryPlanner, query: string) =>
    collectFetches(planner.plan(query, undefined, new Set(), 0).node).filter(
      (fetch) =>
        fetch.contextVariables ||
        fetch.entityBatch?.aliases.some((alias: any) => alias.contextVariables),
    );

  const basic = await supergraph("context-basic");
  const query = "{ users { txs { amount } } }";
  // no limit, every group goes in one request
  const [unlimited] = contextual(new QueryPlanner(basic), query);
  t.assert.strictEqual(unlimited.kind, "Fetch");
  t.assert.ok(!("maxContextGroups" in unlimited));
  const [global] = contextual(new QueryPlanner(basic, { maxAliasesPerRequest: 15 }), query);
  t.assert.strictEqual(global.maxContextGroups, 15);
  const [overridden] = contextual(
    new QueryPlanner(basic, {
      maxAliasesPerRequest: 15,
      subgraphMaxAliasesPerRequest: { a: 4 },
    }),
    query,
  );
  t.assert.strictEqual(overridden.maxContextGroups, 4);
  // fetches without context variables have no limit to follow
  for (const fetch of collectFetches(
    new QueryPlanner(basic, { maxAliasesPerRequest: 15 }).plan(query, undefined, new Set(), 0)
      .node,
  )) {
    t.assert.strictEqual("maxContextGroups" in fetch, !!fetch.contextVariables);
  }

  // a non-null contextual argument can't be aliased per group, so one request per group
  const [nonNull] = contextual(
    new QueryPlanner(await supergraph("context-non-null")),
    "{ viewer { ... on User { txs { amount } } } }",
  );
  t.assert.strictEqual(nonNull.maxContextGroups, 1);

  // every group repeats all aliases of a batch
  const duck = await supergraph("context-shared-duck");
  const duckQuery = "{ foo { u { field } } bar { u { field } } }";
  const [unlimitedBatch] = contextual(new QueryPlanner(duck), duckQuery);
  t.assert.strictEqual(unlimitedBatch.kind, "BatchFetch");
  t.assert.strictEqual(unlimitedBatch.entityBatch.aliases.length, 2);
  t.assert.ok(!("maxContextGroups" in unlimitedBatch));
  const [batch] = contextual(new QueryPlanner(duck, { maxAliasesPerRequest: 15 }), duckQuery);
  t.assert.strictEqual(batch.maxContextGroups, 7);

  t.assert.throws(() => new QueryPlanner(basic, { maxAliasesPerRequest: -1 }), {
    message: "maxAliasesPerRequest must be an integer from 0 to 4294967295, got -1",
  });
  t.assert.throws(
    () => new QueryPlanner(basic, { subgraphMaxAliasesPerRequest: { a: 1.5 } }),
    { message: "subgraphMaxAliasesPerRequest.a must be an integer from 0 to 4294967295, got 1.5" },
  );
});

it("should expose each subgraph's resolvable entity keys", async (t) => {
  const supergraph = await fs.readFile(
    "../../bin/router/fixture/tests/keys-mashup.supergraph.graphql",
    "utf-8",
  );
  // `A` has four keys in each subgraph, only the resolvable one counts, with its top-level fields
  t.assert.deepStrictEqual(new QueryPlanner(supergraph).entityKeys, {
    a: { A: [["id"]] },
    b: { A: [["id", "compositeId"]], B: [["id"]] },
  });

  const typenameKeyed = new QueryPlanner(/* GraphQL */ `
    enum join__Graph {
      CONTENT @join__graph(name: "content", url: "")
      PETSEARCH @join__graph(name: "petsearch", url: "")
    }
    type Query @join__type(graph: CONTENT) {
      favoritesPage: FavoritesPage @join__field(graph: CONTENT)
    }
    type FavoritesPage
      @join__type(graph: CONTENT, key: "__typename")
      @join__type(graph: PETSEARCH, key: "__typename") {
      title: String @join__field(graph: CONTENT)
      petCount: Int @join__field(graph: PETSEARCH)
    }
  `);
  // a key of `__typename` alone has no fields
  t.assert.deepStrictEqual(typenameKeyed.entityKeys, {
    content: { FavoritesPage: [[]] },
    petsearch: { FavoritesPage: [[]] },
  });
});

it("should expose possible types of interfaces and unions", async (t) => {
  const supergraph = await fs.readFile(
    "../../bin/router/fixture/tests/abstract-types.supergraph.graphql",
    "utf-8",
  );
  t.assert.deepStrictEqual(new QueryPlanner(supergraph).possibleTypes, {
    Product: ["Book", "Magazine"],
    PublisherType: ["Agency", "Group", "Self"],
    Similar: ["Book", "Magazine"],
  });
});

it("should order root mutation fields with dependencies", async (t) => {
  const supergraph = await fs.readFile(
    "../../bin/router/fixture/tests/mutations.supergraph.graphql",
    "utf-8",
  );
  const plan = new QueryPlanner(supergraph).plan(
    `mutation {
      addProduct(input: { name: "new", price: 599.99 }) { isExpensive }
      five: add(num: 5)
      ten: multiply(by: 2)
    }`,
    undefined,
    new Set(),
    0,
  );
  const [addProduct, isExpensive, five, ten] = collectFetches(plan.node);
  t.assert.deepStrictEqual(addProduct.dependsOn, []);
  t.assert.deepStrictEqual(isExpensive.dependsOn, [addProduct.id]);
  // the next field waits for every fetch of the one before it, entity calls included
  t.assert.deepStrictEqual(five.dependsOn, [isExpensive.id]);
  t.assert.deepStrictEqual(ten.dependsOn, [five.id]);
});

it("should expose the response projection", async (t) => {
  const planner = new QueryPlanner(/* GraphQL */ `
    enum join__Graph {
      PETS @join__graph(name: "pets", url: "")
    }
    type Query @join__type(graph: PETS) {
      owner: Owner
    }
    type Owner @join__type(graph: PETS) {
      grid: [[Int!]]!
      pets: [Pet!]
      favorite: Animal
    }
    interface Pet @join__type(graph: PETS) {
      name: String!
    }
    type Dog implements Pet
      @join__type(graph: PETS)
      @join__implements(graph: PETS, interface: "Pet") {
      name: String!
      breed: Breed
    }
    type Cat implements Pet
      @join__type(graph: PETS)
      @join__implements(graph: PETS, interface: "Pet") {
      name: String!
      lives: Int
    }
    union Animal
      @join__type(graph: PETS)
      @join__unionMember(graph: PETS, member: "Dog")
      @join__unionMember(graph: PETS, member: "Cat") = Dog | Cat
    enum Breed @join__type(graph: PETS) {
      LAB
      POODLE
    }
  `);
  const plan = planner.plan(
    `query ($withPets: Boolean!) {
      owner {
        __typename
        grid
        pets @include(if: $withPets) {
          name
          ... on Dog { kind: breed }
          ... on Cat { lives }
        }
        favorite { ... on Dog { breed } }
      }
    }`,
    undefined,
    new Set(),
    0,
  );
  t.assert.snapshot(plan.projection);
});
