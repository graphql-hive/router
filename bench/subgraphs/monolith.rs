use async_graphql::{
    ComplexObject, Interface, Object, Schema, SimpleObject, Subscription, ID, InputObject, Upload, Context
};
use futures::stream::{self, Stream};
use std::time::Duration;
use std::io::Read;
use tokio::io::AsyncWriteExt;

// Import the static data from the subgraphs
use crate::accounts::USERS;
use crate::inventory::{INVENTORY};
use crate::products::{PRODUCTS};
use crate::reviews::{REVIEWS};

#[derive(Interface, Clone)]
#[graphql(
    field(name = "url", ty = "String"),
    field(name = "handle", ty = "String")
)]
pub enum SocialAccount {
    TwitterAccount(TwitterAccount),
    GitHubAccount(GitHubAccount),
}

#[derive(SimpleObject, Clone)]
pub struct TwitterAccount {
    url: String,
    handle: String,
    followers: i32,
}

#[derive(SimpleObject, Clone)]
pub struct GitHubAccount {
    url: String,
    handle: String,
    repo_count: i32,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct User {
    id: ID,
    name: Option<String>,
    username: Option<String>,
    birthday: Option<i32>,
}

#[ComplexObject]
impl User {
    async fn social_accounts(&self) -> Vec<SocialAccount> {
        vec![
            SocialAccount::TwitterAccount(TwitterAccount {
                url: format!(
                    "https://twitter.com/{}",
                    self.username.as_ref().unwrap_or(&"unknown".to_string())
                ),
                handle: format!(
                    "@{}",
                    self.username.as_ref().unwrap_or(&"unknown".to_string())
                ),
                followers: 1000,
            }),
            SocialAccount::GitHubAccount(GitHubAccount {
                url: format!(
                    "https://github.com/{}",
                    self.username.as_ref().unwrap_or(&"unknown".to_string())
                ),
                handle: self
                    .username
                    .as_ref()
                    .unwrap_or(&"unknown".to_string())
                    .clone(),
                repo_count: 42,
            }),
        ]
    }

    async fn reviews(&self) -> Option<Vec<Option<Review>>> {
        let relevant = REVIEWS.iter().filter(|r| {
            // we mock this by saying user "1" has reviews 0 and 1
            if self.id.as_str() == "1" {
                r.id.as_str() == "1" || r.id.as_str() == "2"
            } else {
                false
            }
        }).map(|r| Some(Review {
            id: r.id.clone(),
            body: r.body.clone(),
            product: r.product.as_ref().map(|p| Product {
                upc: p.upc.clone(),
            }),
        })).collect();
        Some(relevant)
    }
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct Product {
    pub upc: String,
}

#[ComplexObject]
impl Product {
    async fn name(&self) -> Option<String> {
        PRODUCTS.iter().find(|p| p.upc == self.upc).and_then(|p| p.name.clone())
    }

    async fn price(&self) -> Option<i64> {
        PRODUCTS.iter().find(|p| p.upc == self.upc).and_then(|p| p.price)
    }

    async fn weight(&self) -> Option<i64> {
        PRODUCTS.iter().find(|p| p.upc == self.upc).and_then(|p| p.weight)
    }

    async fn notes(&self) -> Option<String> {
        PRODUCTS.iter().find(|p| p.upc == self.upc).and_then(|p| p.notes.clone())
    }

    async fn internal(&self) -> Option<String> {
        PRODUCTS.iter().find(|p| p.upc == self.upc).and_then(|p| p.internal.clone())
    }

    async fn in_stock(&self) -> Option<bool> {
        INVENTORY.iter().find(|i| i.upc == self.upc).and_then(|i| i.in_stock)
    }

    async fn shipping_estimate(&self) -> Option<i64> {
        let price = PRODUCTS.iter().find(|p| p.upc == self.upc).and_then(|p| p.price);
        let weight = PRODUCTS.iter().find(|p| p.upc == self.upc).and_then(|p| p.weight);

        if let Some(price) = price {
            if price > 1000 {
                return Some(0);
            }
            if let Some(weight) = weight {
                return Some(weight / 2);
            }
        }
        None
    }

    async fn reviews(&self) -> Option<Vec<Option<Review>>> {
        let relevant = REVIEWS
            .iter()
            .filter(|r| {
                if let Some(product) = &r.product {
                    product.upc == self.upc
                } else {
                    false
                }
            })
            .map(|r| Some(Review {
                id: r.id.clone(),
                body: r.body.clone(),
                product: r.product.as_ref().map(|p| Product {
                    upc: p.upc.clone(),
                }),
            }))
            .collect();
        Some(relevant)
    }
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct Review {
    pub id: ID,
    pub body: Option<String>,
    pub product: Option<Product>,
}

#[ComplexObject]
impl Review {
    pub async fn author(&self) -> Option<User> {
        Some(User {
            id: ID("1".to_string()),
            name: Some("Uri Goldshtein".to_string()),
            username: Some("urigo".to_string()),
            birthday: Some(1234567890),
        })
    }
}

pub struct Query;

#[Object]
impl Query {
    async fn me(&self) -> Option<User> {
        Some(User {
            id: USERS[0].id.clone(),
            name: USERS[0].name.clone(),
            username: USERS[0].username.clone(),
            birthday: USERS[0].birthday,
        })
    }

    async fn user(&self, id: ID) -> Option<User> {
        USERS.iter().find(|user| user.id == id).map(|u| User {
            id: u.id.clone(),
            name: u.name.clone(),
            username: u.username.clone(),
            birthday: u.birthday,
        })
    }

    async fn users(&self) -> Option<Vec<Option<User>>> {
        Some(USERS.iter().map(|u| Some(User {
            id: u.id.clone(),
            name: u.name.clone(),
            username: u.username.clone(),
            birthday: u.birthday,
        })).collect())
    }

    async fn top_products(
        &self,
        #[graphql(default = 5)] first: Option<i32>,
    ) -> Option<Vec<Option<Product>>> {
        Some(
            PRODUCTS
                .iter()
                .take(first.unwrap_or(5) as usize)
                .map(|product| Some(Product {
                    upc: product.upc.clone()
                }))
                .collect(),
        )
    }
}

pub struct Mutation;

#[Object]
impl Mutation {
    async fn upload(&self, ctx: &Context<'_>, file: Option<Upload>) -> String {
        if file.is_none() {
            return "No file uploaded".to_string();
        }
        let uploaded_file = file.unwrap().value(ctx).unwrap();
        let path = format!("/tmp/{}", uploaded_file.filename);
        let mut buf = vec![];
        let _ = uploaded_file.into_read().read_to_end(&mut buf);
        let mut tmp_file_on_disk = tokio::fs::File::create(&path).await.unwrap();
        tmp_file_on_disk.write_all(&buf).await.unwrap();
        path
    }
    async fn oneof_test(&self, input: OneOfTestInput) -> OneOfTestResult {
        OneOfTestResult {
            string: input.string,
            int: input.int,
            float: input.float,
            boolean: input.boolean,
            id: input.id,
        }
    }
}

#[derive(InputObject)]
struct OneOfTestInput {
    pub string: Option<String>,
    pub int: Option<i32>,
    pub float: Option<f64>,
    pub boolean: Option<bool>,
    pub id: Option<ID>,
}

#[derive(SimpleObject)]
struct OneOfTestResult {
    pub string: Option<String>,
    pub int: Option<i32>,
    pub float: Option<f64>,
    pub boolean: Option<bool>,
    pub id: Option<ID>,
}

pub struct Subscription;

#[Subscription]
impl Subscription {
    async fn review_added(
        &self,
        #[graphql(default = 1)] step: usize,
        #[graphql(default = 1_000)] interval_in_ms: u64,
    ) -> impl Stream<Item = Review> {
        stream::unfold(
            (
                0,
                if interval_in_ms > 0 {
                    Some(tokio::time::interval(Duration::from_millis(interval_in_ms)))
                } else {
                    None
                },
            ),
            move |(i, mut interval)| async move {
                match REVIEWS.get(i) {
                    Some(review) => {
                        if let Some(int) = &mut interval {
                            int.tick().await;
                        }
                        Some((Review {
                            id: review.id.clone(),
                            body: review.body.clone(),
                            product: review.product.as_ref().map(|p| Product { upc: p.upc.clone() }),
                        }, (i + step, interval)))
                    }
                    None => None,
                }
            },
        )
    }

    async fn review_added_for_product(
        &self,
        product_upc: String,
        #[graphql(default = 1_000)] interval_in_ms: u64,
    ) -> impl Stream<Item = Review> {
        let reviews_for_product: Vec<Review> = REVIEWS
            .iter()
            .filter(move |r| r.product.as_ref().unwrap().upc == product_upc)
            .map(|review| Review {
                id: review.id.clone(),
                body: review.body.clone(),
                product: review.product.as_ref().map(|p| Product { upc: p.upc.clone() }),
            })
            .collect();

        stream::unfold(
            (
                reviews_for_product,
                0,
                if interval_in_ms > 0 {
                    Some(tokio::time::interval(Duration::from_millis(interval_in_ms)))
                } else {
                    None
                },
            ),
            move |(reviews_for_product, i, mut interval): (Vec<Review>, usize, Option<tokio::time::Interval>)| async move {
                match reviews_for_product.get(i) {
                    Some(review) => {
                        if let Some(int) = &mut interval {
                            int.tick().await;
                        }
                        Some((review.clone(), (reviews_for_product, i + 1, interval)))
                    }
                    None => None,
                }
            },
        )
    }
}

pub fn get_schema() -> Schema<Query, Mutation, Subscription> {
    Schema::build(Query, Mutation, Subscription).finish()
}
