use async_graphql::{EmptySubscription, InputObject, Object, Schema, SimpleObject, ID};
use lazy_static::lazy_static;

lazy_static! {
    static ref BOOKS: Vec<Book> = vec![
        Book {
            title: "The Mystery at Midnight".to_string(),
            genre: BookGenre::Mystery,
            id: BookId("1".to_string()),
            author: Author {
                name: "Alice Smith".to_string(),
                bio: Some("A bestselling author.".to_string()),
            },
            publisher: Publisher {
                name: "Penguin Books".to_string(),
                address: Address { zip_code: 10001 },
                address_with_cost: CostlyAddress { zip_code: 10001 },
            },
        },
        Book {
            title: "Science Fiction Dreams".to_string(),
            genre: BookGenre::Scifi,
            id: BookId("2".to_string()),
            author: Author {
                name: "Bob Johnson".to_string(),
                bio: Some("A visionary writer.".to_string()),
            },
            publisher: Publisher {
                name: "Simon & Schuster".to_string(),
                address: Address { zip_code: 10002 },
                address_with_cost: CostlyAddress { zip_code: 10002 },
            },
        },
        Book {
            title: "Classic Fiction".to_string(),
            genre: BookGenre::Fiction,
            id: BookId("3".to_string()),
            author: Author {
                name: "Charlie Brown".to_string(),
                bio: Some("A timeless novelist.".to_string()),
            },
            publisher: Publisher {
                name: "Harper Collins".to_string(),
                address: Address { zip_code: 10003 },
                address_with_cost: CostlyAddress { zip_code: 10003 },
            },
        },
        Book {
            title: "Murder in the Library".to_string(),
            genre: BookGenre::Mystery,
            id: BookId("4".to_string()),
            author: Author {
                name: "Diana Prince".to_string(),
                bio: Some("Mystery specialist.".to_string()),
            },
            publisher: Publisher {
                name: "Penguin Books".to_string(),
                address: Address { zip_code: 10004 },
                address_with_cost: CostlyAddress { zip_code: 10004 },
            },
        },
        Book {
            title: "Future Worlds".to_string(),
            genre: BookGenre::Scifi,
            id: BookId("5".to_string()),
            author: Author {
                name: "Eve Wilson".to_string(),
                bio: Some("Sci-fi pioneer.".to_string()),
            },
            publisher: Publisher {
                name: "Orbit".to_string(),
                address: Address { zip_code: 10005 },
                address_with_cost: CostlyAddress { zip_code: 10005 },
            },
        },
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, async_graphql::Enum)]
#[graphql(name = "BookGenre")]
pub enum BookGenre {
    Fiction,
    Mystery,
    Scifi,
}

#[derive(Clone, Debug)]
pub struct BookId(pub String);

#[async_graphql::Scalar(name = "BookId")]
impl async_graphql::ScalarType for BookId {
    fn parse(value: async_graphql::Value) -> async_graphql::InputValueResult<Self> {
        match value {
            async_graphql::Value::String(s) => Ok(BookId(s)),
            _ => Err(async_graphql::InputValueError::expected_type(value)),
        }
    }

    fn to_value(&self) -> async_graphql::Value {
        async_graphql::Value::String(self.0.clone())
    }
}

#[derive(SimpleObject, Clone)]
pub struct Address {
    #[graphql(name = "zipCode")]
    zip_code: i32,
}

#[derive(SimpleObject, Clone)]
pub struct CostlyAddress {
    #[graphql(name = "zipCode")]
    zip_code: i32,
}

#[derive(SimpleObject, Clone)]
pub struct Author {
    name: String,
    bio: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct Publisher {
    name: String,
    address: Address,
    #[graphql(name = "addressWithCost")]
    address_with_cost: CostlyAddress,
}

#[derive(SimpleObject, Clone)]
#[graphql(name = "Book")]
pub struct Book {
    title: String,
    genre: BookGenre,
    id: BookId,
    author: Author,
    publisher: Publisher,
}

#[derive(SimpleObject, Clone)]
pub struct Cursor {
    page: Vec<Book>,
    #[graphql(name = "nextPage")]
    next_page: ID,
}

#[derive(SimpleObject, Clone)]
pub struct ResultPage {
    books: Vec<Book>,
    metadata: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ResultContainer {
    page: Vec<Book>,
    recent: Option<Vec<Option<Book>>>,
    metadata: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct DeepContainer {
    results: ResultContainer,
}

#[derive(InputObject, Clone)]
pub struct PaginationInput {
    first: Option<i32>,
}

#[derive(InputObject, Clone)]
pub struct SearchInput {
    pagination: Option<PaginationInput>,
}

#[derive(InputObject, Clone)]
pub struct Level2PaginationInput {
    count: Option<i32>,
}

#[derive(InputObject, Clone)]
pub struct Level1PaginationInput {
    level2: Option<Level2PaginationInput>,
}

#[derive(InputObject, Clone)]
pub struct DeepPaginationInput {
    level1: Option<Level1PaginationInput>,
}

#[derive(InputObject, Clone)]
pub struct CostlySearchInput {
    query: Option<String>,
    limit: Option<i32>,
}

pub struct Query;

#[Object(extends = true)]
impl Query {
    async fn book(&self, id: Option<ID>) -> Option<Book> {
        let search_id = id.map(|i| i.to_string());
        BOOKS
            .iter()
            .find(|b| Some(b.id.0.clone()) == search_id)
            .cloned()
    }

    async fn book_with_field_cost(&self) -> Book {
        BOOKS[0].clone()
    }

    async fn book_with_arg_cost(&self, _limit: Option<i32>) -> Book {
        BOOKS[0].clone()
    }

    async fn bestsellers(&self) -> Vec<Book> {
        BOOKS.iter().take(5).cloned().collect()
    }

    async fn newest_additions(&self, _after: Option<ID>, limit: i32) -> Vec<Book> {
        BOOKS.iter().take(limit as usize).cloned().collect()
    }

    async fn newest_additions_2(&self, first: Option<i32>, last: Option<i32>) -> Vec<Book> {
        let size = std::cmp::max(first.unwrap_or(1), last.unwrap_or(1)) as usize;
        BOOKS.iter().take(size).cloned().collect()
    }

    async fn newest_additions_by_cursor(&self, limit: i32) -> Cursor {
        Cursor {
            page: BOOKS.iter().take(limit as usize).cloned().collect(),
            next_page: ID::from("next"),
        }
    }

    async fn search(&self, input: SearchInput) -> Vec<Book> {
        let limit = input.pagination.and_then(|p| p.first).unwrap_or(10) as usize;
        BOOKS.iter().take(limit).cloned().collect()
    }

    async fn deep_search(&self, input: DeepPaginationInput) -> Vec<Book> {
        let limit = input
            .level1
            .and_then(|l1| l1.level2)
            .and_then(|l2| l2.count)
            .unwrap_or(10) as usize;
        BOOKS.iter().take(limit).cloned().collect()
    }

    async fn deep_container(&self, _first: Option<i32>) -> DeepContainer {
        DeepContainer {
            results: ResultContainer {
                page: BOOKS.clone().into_iter().collect(),
                recent: Some(BOOKS.iter().map(|b| Some(b.clone())).collect()),
                metadata: Some("metadata".to_string()),
            },
        }
    }

    async fn search_by_costly_input(&self, input: CostlySearchInput) -> Vec<Book> {
        let limit = input.limit.unwrap_or(10) as usize;
        let books = BOOKS.iter().take(limit).cloned();

        match input.query {
            Some(query) => {
                let query = query.to_lowercase();
                books
                    .filter(|book| book.title.to_lowercase().contains(&query))
                    .collect()
            }
            None => books.collect(),
        }
    }

    async fn books_by_genre(&self, genre: BookGenre) -> Vec<Book> {
        BOOKS
            .iter()
            .filter(|b| b.genre as i32 == genre as i32)
            .cloned()
            .collect()
    }

    async fn ping(&self) -> String {
        "pong".to_string()
    }
}

pub struct Mutation;

#[Object(extends = true)]
impl Mutation {
    async fn do_thing(&self) -> String {
        "done".to_string()
    }
}

pub fn get_subgraph() -> Schema<Query, Mutation, EmptySubscription> {
    Schema::build(Query, Mutation, EmptySubscription)
        .enable_federation()
        .finish()
}
