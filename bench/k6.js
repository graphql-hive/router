import http from "k6/http";
import { check } from "k6";
import { textSummary } from "https://jslib.k6.io/k6-summary/0.0.1/index.js";
import { githubComment } from "https://raw.githubusercontent.com/dotansimha/k6-github-pr-comment/master/lib.js";

const vus = __ENV.BENCH_VUS ? parseInt(__ENV.BENCH_VUS) : 50;
const time = __ENV.BENCH_OVER_TIME || "30s";

export const options = {
  vus: vus,
  duration: time,
};

export function setup() {
  for (let i = 0; i < 20; i++) {
    sendGraphQLRequest();
  }
}

export default function () {
  makeGraphQLRequest();
}

export function handleSummary(data) {
  if (__ENV.GITHUB_TOKEN) {
    githubComment(data, {
      token: __ENV.GITHUB_TOKEN,
      commit: __ENV.GITHUB_SHA,
      pr: __ENV.GITHUB_PR,
      org: "graphql-hive",
      repo: "gateway-rs",
      commentKey: `k6-benchmark`,
      renderTitle({ passes }) {
        return passes
          ? `✅ \`k6-benchmark\` results`
          : `❌ \`k6-benchmark\` failed`;
      },
      renderMessage({ passes, checks, thresholds }) {
        const result = [];

        if (thresholds.failures) {
          result.push(`**Performance regression detected**`);
        }

        if (checks.failures) {
          result.push("**Failed assertions detected**");
        }

        if (!passes) {
          result.push(
            `> If the performance regression is expected, please increase the failing threshold.`
          );
        }

        return result.join("\n");
      },
    });
  }
  return handleBenchmarkSummary(data, { vus, time });
}

let identifiersMap = {};

function printOnce(identifier, ...args) {
  if (identifiersMap[identifier]) {
    return;
  }

  console.log(...args);
  identifiersMap[identifier] = true;
}

const graphqlRequest = {
  payload: JSON.stringify({
    query: `fragment User on User {
      id
      username
      name
    }

    fragment Review on Review {
      id
      body
    }

    fragment Product on Product {
      inStock
      name
      price
      shippingEstimate
      upc
      weight
    }

    query TestQuery {
      users {
        ...User
        reviews {
          ...Review
          product {
            ...Product
            reviews {
              ...Review
              author {
                ...User
                reviews {
                  ...Review
                  product {
                    ...Product
                  }
                }
              }
            }
          }
        }
      }
      topProducts {
        ...Product
        reviews {
          ...Review
          author {
            ...User
            reviews {
              ...Review
              product {
                ...Product
              }
            }
          }
        }
      }
    }`,
  }),
  params: {
    headers: {
      "Content-Type": "application/json",
    },
  },
};

function handleBenchmarkSummary(data, additionalContext = {}) {
  const out = {
    stdout: textSummary(data, { indent: " ", enableColors: true }),
  };

  if (__ENV.SUMMARY_PATH) {
    out[`${__ENV.SUMMARY_PATH}/k6_summary.json`] = JSON.stringify(
      Object.assign(data, additionalContext),
    );
    out[`${__ENV.SUMMARY_PATH}/k6_summary.txt`] = textSummary(data, {
      indent: " ",
      enableColors: false,
    });
  }

  return out;
}

function sendGraphQLRequest() {
  const res = http.post(
    __ENV.GATEWAY_ENDPOINT || "http://0.0.0.0:4000/graphql",
    graphqlRequest.payload,
    graphqlRequest.params,
  );

  if (res.status !== 200) {
    console.log(`‼️ Failed to run HTTP request:`, res);
  }

  return res;
}

function makeGraphQLRequest() {
  const res = sendGraphQLRequest();
  check(res, {
    "response code was 200": (res) => res.status == 200,
    "no graphql errors": (resp) => {
      const json = resp.json();
      const noErrors =
        !!json &&
        typeof json === "object" &&
        !Array.isArray(json) &&
        !json.errors;

      if (!noErrors) {
        printOnce(
          "graphql_errors",
          `‼️ Got GraphQL errors, here's a sample:`,
          res.body,
        );
      }

      return noErrors;
    },
    "valid response structure": (resp) => {
      const json = resp.json();

      let isValid = checkResponseStructure(json);

      if (!isValid) {
        printOnce(
          "response_strcuture",
          `‼️ Got invalid structure, here's a sample:`,
          res.body,
        );
      }

      return isValid;
    },
  });
}

function checkResponseStructure(x) {
  function checkRecursive(obj, structure) {
    if (obj == null) {
      return false;
    }
    for (var key in structure) {
      if (
        !obj.hasOwnProperty(key) ||
        typeof obj[key] !== typeof structure[key]
      ) {
        return false;
      }
      if (typeof structure[key] === "object" && structure[key] !== null) {
        if (!checkRecursive(obj[key], structure[key])) {
          return false;
        }
      }
    }
    return true;
  }

  const expectedStructure = {
    data: {
      users: [
        {
          id: "1",
          username: "urigo",
          name: "Uri Goldshtein",
          reviews: [
            {
              id: "1",
              body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
            {
              id: "2",
              body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          id: "2",
          username: "dotansimha",
          name: "Dotan Simha",
          reviews: [
            {
              id: "1",
              body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
            {
              id: "2",
              body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          id: "3",
          username: "kamilkisiela",
          name: "Kamil Kisiela",
          reviews: [
            {
              id: "1",
              body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
            {
              id: "2",
              body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          id: "4",
          username: "ardatan",
          name: "Arda Tanrikulu",
          reviews: [
            {
              id: "1",
              body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
            {
              id: "2",
              body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          id: "5",
          username: "gilgardosh",
          name: "Gil Gardosh",
          reviews: [
            {
              id: "1",
              body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
            {
              id: "2",
              body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          id: "6",
          username: "laurin",
          name: "Laurin Quast",
          reviews: [
            {
              id: "1",
              body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
            {
              id: "2",
              body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
              product: {
                inStock: true,
                name: "Table",
                price: 899,
                shippingEstimate: null,
                upc: "1",
                weight: 100,
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "3",
                    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                  {
                    id: "4",
                    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
                    author: {
                      id: "1",
                      username: "urigo",
                      name: "Uri Goldshtein",
                      reviews: [
                        {
                          id: "1",
                          body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                        {
                          id: "2",
                          body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                          product: {
                            inStock: true,
                            name: "Table",
                            price: 899,
                            shippingEstimate: null,
                            upc: "1",
                            weight: 100,
                          },
                        },
                      ],
                    },
                  },
                ],
              },
            },
          ],
        },
      ],
      topProducts: [
        {
          inStock: true,
          name: "Table",
          price: 899,
          shippingEstimate: null,
          upc: "1",
          weight: 100,
          reviews: [
            {
              id: "1",
              body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
            {
              id: "2",
              body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
            {
              id: "3",
              body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
            {
              id: "4",
              body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          inStock: false,
          name: "Couch",
          price: 1299,
          shippingEstimate: null,
          upc: "2",
          weight: 1000,
          reviews: [
            {
              id: "5",
              body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
            {
              id: "6",
              body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
            {
              id: "7",
              body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
            {
              id: "8",
              body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          inStock: false,
          name: "Glass",
          price: 15,
          shippingEstimate: null,
          upc: "3",
          weight: 20,
          reviews: [
            {
              id: "9",
              body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          inStock: false,
          name: "Chair",
          price: 499,
          shippingEstimate: null,
          upc: "4",
          weight: 100,
          reviews: [
            {
              id: "10",
              body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
            {
              id: "11",
              body: "At vero eos et accusamus et iusto odio dignissimos ducimus qui blanditiis praesentium voluptatum deleniti atque corrupti quos dolores et quas molestias excepturi sint occaecati cupiditate non provident, similique sunt in culpa qui officia deserunt mollitia animi, id est laborum et dolorum fuga. Et harum quidem rerum facilis est et expedita distinctio. Nam libero tempore, cum soluta nobis est eligendi optio cumque nihil impedit quo minus id quod maxime placeat facere possimus, omnis voluptas assumenda est, omnis dolor repellendus. Temporibus autem quibusdam et aut officiis debitis aut rerum necessitatibus saepe eveniet ut et voluptates repudiandae sint et molestiae non recusandae. Itaque earum rerum hic tenetur a sapiente delectus, ut aut reiciendis voluptatibus maiores alias consequatur aut perferendis doloribus asperiores repellat.",
              author: {
                id: "1",
                username: "urigo",
                name: "Uri Goldshtein",
                reviews: [
                  {
                    id: "1",
                    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                  {
                    id: "2",
                    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
                    product: {
                      inStock: true,
                      name: "Table",
                      price: 899,
                      shippingEstimate: null,
                      upc: "1",
                      weight: 100,
                    },
                  },
                ],
              },
            },
          ],
        },
        {
          inStock: true,
          name: "TV",
          price: 1299,
          shippingEstimate: null,
          upc: "5",
          weight: 1000,
          reviews: [],
        },
      ],
    },
  };
  return checkRecursive(x, expectedStructure);
}
