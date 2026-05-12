import { createYoga, createSchema } from "graphql-yoga";
import { createServer } from "http";
import { typeDefs } from "./schema.js";
import { resolvers } from "./resolvers.js";

const schema = createSchema({
  typeDefs,
  resolvers,
});

const yoga = createYoga({
  schema,
});

const server = createServer(yoga);

const PORT = process.env.PORT || 4300;

server.listen(PORT, () => {
  console.log(
    `🚀 GraphQL Yoga monolith server running at http://localhost:${PORT}/graphql`,
  );
});
