// Static data for the monolith
export const USERS = [
  {
    id: "1",
    name: "Uri Goldshtein",
    username: "urigo",
    birthday: 1234567890,
  },
  {
    id: "2",
    name: "Dotan Simha",
    username: "dotansimha",
    birthday: 1234567890,
  },
  {
    id: "3",
    name: "Kamil Kisiela",
    username: "kamilkisiela",
    birthday: 1234567890,
  },
  {
    id: "4",
    name: "Arda Tanrikulu",
    username: "ardatan",
    birthday: 1234567890,
  },
  {
    id: "5",
    name: "Gil Gardosh",
    username: "gilgardosh",
    birthday: 1234567890,
  },
  {
    id: "6",
    name: "Laurin Quast",
    username: "laurin",
    birthday: 1234567890,
  },
];

export const PRODUCTS = [
  {
    upc: "1",
    name: "Table",
    price: 899,
    weight: 100,
    notes: "Notes for table",
    internal: "Internal for table",
  },
  {
    upc: "2",
    name: "Couch",
    price: 1299,
    weight: 1000,
    notes: "Notes for couch",
    internal: "Internal for couch",
  },
  {
    upc: "3",
    name: "Glass",
    price: 15,
    weight: 20,
    notes: "Notes for glass",
    internal: "Internal for glass",
  },
  {
    upc: "4",
    name: "Chair",
    price: 499,
    weight: 100,
    notes: "Notes for chair",
    internal: "Internal for chair",
  },
  {
    upc: "5",
    name: "TV",
    price: 1299,
    weight: 1000,
    notes: "Notes for TV",
    internal: "Internal for TV",
  },
  {
    upc: "6",
    name: "Lamp",
    price: 6999,
    weight: 300,
    notes: "Notes for lamp",
    internal: "Internal for lamp",
  },
  {
    upc: "7",
    name: "Grill",
    price: 3999,
    weight: 2000,
    notes: "Notes for grill",
    internal: "Internal for grill",
  },
  {
    upc: "8",
    name: "Fridge",
    price: 100000,
    weight: 6000,
    notes: "Notes for fridge",
    internal: "Internal for fridge",
  },
  {
    upc: "9",
    name: "Sofa",
    price: 9999,
    weight: 800,
    notes: "Notes for sofa",
    internal: "Internal for sofa",
  },
];

export const INVENTORY = [
  { upc: "1", in_stock: true },
  { upc: "2", in_stock: false },
  { upc: "3", in_stock: false },
  { upc: "4", in_stock: false },
  { upc: "5", in_stock: true },
  { upc: "6", in_stock: true },
  { upc: "7", in_stock: true },
  { upc: "8", in_stock: false },
  { upc: "9", in_stock: true },
];

export const REVIEWS = [
  {
    id: "1",
    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
    product: { upc: "1" },
  },
  {
    id: "2",
    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
    product: { upc: "1" },
  },
  {
    id: "3",
    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
    product: { upc: "1" },
  },
  {
    id: "4",
    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
    product: { upc: "1" },
  },
  {
    id: "5",
    body: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.",
    product: { upc: "2" },
  },
  {
    id: "6",
    body: "Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugi",
    product: { upc: "2" },
  },
  {
    id: "7",
    body: "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt. Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem.",
    product: { upc: "2" },
  },
  {
    id: "8",
    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
    product: { upc: "2" },
  },
  {
    id: "9",
    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
    product: { upc: "3" },
  },
  {
    id: "10",
    body: "Ut enim ad minima veniam, quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur? Quis autem",
    product: { upc: "4" },
  },
  {
    id: "11",
    body: "At vero eos et accusamus et iusto odio dignissimos ducimus qui blanditiis praesentium voluptatum deleniti atque corrupti quos dolores et quas molestias excepturi sint occaecati cupiditate non provident, similique sunt in culpa qui officia deserunt mollitia animi, id est laborum et dolorum fuga. Et harum quidem rerum facilis est et expedita distinctio. Nam libero tempore, cum soluta nobis est eligendi optio cumque nihil impedit quo minus id quod maxime placeat facere possimus, omnis voluptas assumenda est, omnis dolor repellendus. Temporibus autem quibusdam et aut officiis debitis aut rerum necessitatibus saepe eveniet ut et voluptates repudiandae sint et molestiae non recusandae. Itaque earum rerum hic tenetur a sapiente delectus, ut aut reiciendis voluptatibus maiores alias consequatur aut perferendis doloribus asperiores repellat.",
    product: { upc: "4" },
  },
];
