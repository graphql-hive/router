use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema, SimpleObject, ID};
use lazy_static::lazy_static;

lazy_static! {
    static ref PRODUCTS: Vec<Product> = vec![
        Product {
            upc: "1".to_string(),
            name: Some("Table".to_string()),
            price: Some(899),
            weight: Some(100),
        },
        Product {
            upc: "2".to_string(),
            name: Some("Couch".to_string()),
            price: Some(1299),
            weight: Some(1000),
        },
        Product {
            upc: "3".to_string(),
            name: Some("Glass".to_string()),
            price: Some(15),
            weight: Some(20),
        },
        Product {
            upc: "4".to_string(),
            name: Some("Chair".to_string()),
            price: Some(499),
            weight: Some(100),
        },
        Product {
            upc: "5".to_string(),
            name: Some("TV".to_string()),
            price: Some(1299),
            weight: Some(1000),
        },
        Product {
            upc: "6".to_string(),
            name: Some("Lamp".to_string()),
            price: Some(6999),
            weight: Some(300),
        },
        Product {
            upc: "7".to_string(),
            name: Some("Grill".to_string()),
            price: Some(3999),
            weight: Some(2000),
        },
        Product {
            upc: "8".to_string(),
            name: Some("Fridge".to_string()),
            price: Some(100000),
            weight: Some(6000),
        },
        Product {
            upc: "9".to_string(),
            name: Some("Sofa".to_string()),
            price: Some(9999),
            weight: Some(800),
        }
    ];
}
#[derive(SimpleObject, Clone)]
pub struct Product {
    upc: String,
    name: Option<String>,
    price: Option<i64>,
    weight: Option<i64>,
}

pub struct Query;

#[Object(extends = true)]
impl Query {
    async fn top_products(
        &self,
        #[graphql(default = 5)] first: Option<i32>,
    ) -> Option<Vec<Option<Product>>> {
        Some(
            PRODUCTS
                .iter()
                .take(first.unwrap_or(5) as usize)
                .map(|product| Some(product.clone()))
                .collect(),
        )
    }

    #[graphql(entity)]
    async fn find_product_by_upc(&self, upc: ID) -> Product {
        PRODUCTS
            .iter()
            .find(|product| product.upc == upc.as_str())
            .unwrap()
            .clone()
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
