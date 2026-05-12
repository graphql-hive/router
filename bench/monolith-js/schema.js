export const typeDefs = `
  interface SocialAccount {
    url: String!
    handle: String!
  }

  type TwitterAccount implements SocialAccount {
    url: String!
    handle: String!
    followers: Int!
  }

  type GitHubAccount implements SocialAccount {
    url: String!
    handle: String!
    repo_count: Int!
  }

  type User {
    id: ID!
    name: String
    username: String
    birthday: Int
    social_accounts: [SocialAccount!]!
    reviews: [Review!]!
  }

  type Product {
    upc: String!
    name: String
    price: Int
    weight: Int
    notes: String
    internal: String
    inStock: Boolean
    shippingEstimate: Int
    reviews: [Review!]!
  }

  type Review {
    id: ID!
    body: String
    product: Product
    author: User!
  }

  type Query {
    me: User
    user(id: ID!): User
    users: [User!]!
    topProducts(first: Int = 5): [Product!]!
  }
`;
