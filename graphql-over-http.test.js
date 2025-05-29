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
    it('preserve the order of the selection set', async () => {
        const response = await fetch("http://localhost:4000/graphql", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
          },
          body: JSON.stringify({
            query: /* GraphQL */ `
                    query {
                      a: __typename
                      __typename
                      __schema {
                        __typename
                      }
                    }
              `,
          }),
        });
        const text = await response.text();
        assert.match(
          text,
          new RegExp(
            '"a":"Query","__typename":"Query","__schema":{"__typename":"__Schema"'
          ),
          "Order of selection set is not preserved"
        );
    })
})