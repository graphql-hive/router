import { USERS, PRODUCTS, INVENTORY, REVIEWS } from "./data.js";

export const resolvers = {
  Query: {
    me: () => USERS[0],

    user: (_, { id }) => {
      return USERS.find((u) => u.id === id);
    },

    users: () => USERS,

    topProducts: (_, { first = 5 }) => {
      return PRODUCTS.slice(0, first);
    },
  },

  User: {
    reviews: (user) => {
      // Mirror the reviews subgraph entity resolver exactly.
      return REVIEWS.slice(0, 2);
    },
  },

  Product: {
    name: (product) => {
      const p = PRODUCTS.find((pr) => pr.upc === product.upc);
      return p?.name;
    },

    price: (product) => {
      const p = PRODUCTS.find((pr) => pr.upc === product.upc);
      return p?.price;
    },

    weight: (product) => {
      const p = PRODUCTS.find((pr) => pr.upc === product.upc);
      return p?.weight;
    },

    notes: (product) => {
      const p = PRODUCTS.find((pr) => pr.upc === product.upc);
      return p?.notes;
    },

    internal: (product) => {
      const p = PRODUCTS.find((pr) => pr.upc === product.upc);
      return p?.internal;
    },

    inStock: (product) => {
      const i = INVENTORY.find((inv) => inv.upc === product.upc);
      return i?.in_stock;
    },

    shippingEstimate: (product) => {
      const p = PRODUCTS.find((pr) => pr.upc === product.upc);
      if (!p) return null;

      if (p.price && p.price > 1000) {
        return 0;
      }

      if (p.price && p.weight) {
        return p.weight / 2;
      }

      return null;
    },

    reviews: (product) => {
      return REVIEWS.filter((r) => r.product?.upc === product.upc);
    },
  },

  Review: {
    author: () => {
      // Always return the first user
      return USERS[0];
    },

    product: (review) => {
      return review.product;
    },
  },
};
