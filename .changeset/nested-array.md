---
router: patch
executor: patch
---

Handle nested abstract items while projecting correctly;

For the following schema;

```graphql
interface IContent {
  id: ID!
}

interface IElementContent {
  id: ID!
}

type ContentAChild {
  title: String!
}

type ContentA implements IContent & IElementContent {
  id: ID!
  contentChildren: [ContentAChild!]!
}

type ContentBChild {
  title: String!
}

type ContentB implements IContent & IElementContent {
  id: ID!
  contentChildren: [ContentBChild!]!
}

type Query {
  contentPage: ContentPage!
}

type ContentPage {
  contentBody: [ContentContainer!]!
}

type ContentContainer {
  id: ID!
  section: IContent
}
```

```graphql
query {
  contentPage {
    contentBody {
      section {
        ...ContentAData
        ...ContentBData
      }
    }
  }
}

fragment ContentAData on ContentA {
  contentChildren {
    title
  }
}

fragment ContentBData on ContentB {
  contentChildren {
    title
  }
}
```

If a query like above is executed, the projection plan should be able to handle nested abstract types correctly.

For the following subgraph response, array items should be handled by their own `__typename` values individually;

```json
{
  "__typename": "Query",
  "contentPage": [
    {
      "__typename": "ContentPage",
      "contentBody": [
        {
          "__typename": "ContentContainer",
          "id": "container1",
          "section": {
            "__typename": "ContentA",
            "contentChildren": []
          }
        },
        {
          "__typename": "ContentContainer",
          "id": "container2",
          "section": {
            "__typename": "ContentB",
            "contentChildren": [
              {
                "__typename": "ContentBChild",
                "title": "contentBChild1"
              }
            ]
          }
        }
      ]
    }
  ]
}
```

On the other hand if parent types of those don't have `__typename`, we don't need to check the parent types while projecting nested abstract items. In this case, the data to be projected would be;

```json
{
  "contentPage": {
    "contentBody": [
      {
        "id": "container1",
        "section": {
          "__typename": "ContentA",
          "id": "contentA1",
          "contentChildren": []
        }
      },
      {
        "id": "container2",
        "section": {
          "__typename": "ContentB",
          "id": "contentB1",
          "contentChildren": null
        }
      }
    ]
  }
}
```
