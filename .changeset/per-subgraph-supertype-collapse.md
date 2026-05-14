---
hive-router-query-planner: patch
node-addon: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Collapse per-implementor inline fragments back to the supertype they came from

When the supergraph has a union of three notification types and a feed field that returns it:

```graphql
interface Notification {
  id: ID!
  title: String
  createdAt: String
}

type EmailNotification implements Notification {
  id: ID!
  title: String
  createdAt: String
  recipientAddress: String
}

type PushNotification implements Notification {
  id: ID!
  title: String
  createdAt: String
  deviceToken: String
}

type SmsNotification implements Notification {
  id: ID!
  title: String
  createdAt: String
  phoneNumber: String
}

union NotificationUnion =
  | EmailNotification
  | PushNotification
  | SmsNotification

type Query {
  notificationFeed: [NotificationUnion!]!
}
```

And the client sends an interface-shaped query against the union:

```graphql
{
  notificationFeed {
    ... on Notification {
      id
      title
      createdAt
    }
  }
}
```

Before, the planner sent this exploded shape to the subgraph:

```graphql
{
  notificationFeed {
    __typename
    ... on EmailNotification {
      id
      title
      createdAt
    }
    ... on PushNotification {
      id
      title
      createdAt
    }
    ... on SmsNotification {
      id
      title
      createdAt
    }
  }
}
```

After, the planner sends back the abstract shape the client wrote:

```graphql
{
  notificationFeed {
    __typename
    ... on Notification {
      id
      title
      createdAt
    }
  }
}
```

The same collapse also applies to entity fetches. When `Product` is an interface implemented by `Book` and `Magazine`, and `reviews` is resolved from another subgraph. The `_entities` representations and the entity fetch payload used to be sent like this:

```graphql
{
  ... on Book {
    __typename
    id
  }
  ... on Magazine {
    __typename
    id
  }
} =>
{
  ... on Book {
    reviews {
      id
    }
  }
  ... on Magazine {
    reviews {
      id
    }
  }
}
```

And now they get sent like this:

```graphql
{
  ... on Product {
    __typename
    id
  }
} =>
{
  ... on Product {
    reviews {
      id
    }
  }
}
```

The collapse also runs independently on each leg when an interface is split across subgraphs. When one subgraph dispatches the runtime objects of an `Item` interface through a root field, while two other subgraphs each own a subset of the implementors and resolve a per-implementor `details` field for them:

```graphql
# directory subgraph
interface Item {
  id: ID!
  details: String
}

type ItemA implements Item { id: ID! }
type ItemB implements Item { id: ID! }
type ItemC implements Item { id: ID! }
type ItemD implements Item { id: ID! }
type ItemE implements Item { id: ID! }
type ItemF implements Item { id: ID! }

type Query {
  allItems: [Item!]!
}

# stream_a subgraph
interface Item {
  id: ID!
  details: String
}

type ItemA implements Item { id: ID! details: String }
type ItemB implements Item { id: ID! details: String }
type ItemC implements Item { id: ID! details: String }

# stream_b subgraph
interface Item {
  id: ID!
  details: String
}

type ItemD implements Item { id: ID! details: String }
type ItemE implements Item { id: ID! details: String }
type ItemF implements Item { id: ID! details: String }
```

And the client sends an interface-shaped query that asks for a field whose resolution is split across the two stream subgraphs:

```graphql
{
  allItems {
    id
    details
  }
}
```

Before, each entity fetch leg had its own per-implementor expansion, even though every implementor in that leg was already covered by `Item` in the target subgraph:

```graphql
# stream_a leg
{
  ... on ItemA { __typename id }
  ... on ItemB { __typename id }
  ... on ItemC { __typename id }
} =>
{
  ... on ItemA { details }
  ... on ItemB { details }
  ... on ItemC { details }
}

# stream_b leg
{
  ... on ItemD { __typename id }
  ... on ItemE { __typename id }
  ... on ItemF { __typename id }
} =>
{
  ... on ItemD { details }
  ... on ItemE { details }
  ... on ItemF { details }
}
```

After, each leg collapses into a single `... on Item` fragment, scoped to that subgraph's implementor set on the router-side runtime path filter (`allItems.@|[ItemA|ItemB|ItemC]` and `allItems.@|[ItemD|ItemE|ItemF]`):

```graphql
# stream_a leg
{
  ... on Item { __typename id }
} =>
{
  ... on Item { details }
}

# stream_b leg
{
  ... on Item { __typename id }
} =>
{
  ... on Item { details }
}
```
