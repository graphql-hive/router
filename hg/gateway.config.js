export const gatewayConfig = {
  additionalTypeDefs: /* GraphQL */ `
    extend type Review {
      favProduct: Product
    }

    # Can't extend Product here as it would lead to conflicting definitions
    # extend type Product {
    #   upc: String!
    # }
  `,
  additionalResolvers: {
    Review: {
      favProduct: () => {
        return { __typename: "Product", upc: "1" };
      },
    },
  },
};
