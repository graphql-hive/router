import { describe, it } from "node:test";
import { serverAudits } from "graphql-http";
import assert from "node:assert";
import { fetch } from '@whatwg-node/fetch';

describe("GraphQL over HTTP", () => {
    for (const audit of serverAudits({
      url: "http://localhost:4000/graphql",
      fetchFn: fetch,
    })) {
      it(audit.name, async () => {
        const result = await audit.fn();
        assert.equal(result.status, "ok", result.reason);
      });
    }
})