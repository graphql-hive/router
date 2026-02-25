use std::io::Read;

use async_graphql::{
    Context, EmptySubscription, InputObject, Object, Schema, SimpleObject, Upload, ID,
};
use lazy_static::lazy_static;
use tokio::io::AsyncWriteExt;

lazy_static! {
    static ref PRODUCTS: Vec<Product> = vec![
        Product {
            upc: "1".to_string(),
            name: Some("Table".to_string()),
            price: Some(899),
            weight: Some(100),
            notes: Some("Notes for table".to_string()),
            internal: Some("Internal for table".to_string()),
        },
        Product {
            upc: "2".to_string(),
            name: Some("Couch".to_string()),
            price: Some(1299),
            weight: Some(1000),
            notes: Some("Notes for couch".to_string()),
            internal: Some("Internal for couch".to_string()),
        },
        Product {
            upc: "3".to_string(),
            name: Some("Glass".to_string()),
            price: Some(15),
            weight: Some(20),
            notes: Some("Notes for glass".to_string()),
            internal: Some("Internal for glass".to_string()),
        },
        Product {
            upc: "4".to_string(),
            name: Some("Chair".to_string()),
            price: Some(499),
            weight: Some(100),
            notes: Some("Notes for chair".to_string()),
            internal: Some("Internal for chair".to_string()),
        },
        Product {
            upc: "5".to_string(),
            name: Some("TV".to_string()),
            price: Some(1299),
            weight: Some(1000),
            notes: Some("Notes for TV".to_string()),
            internal: Some("Internal for TV".to_string()),
        },
        Product {
            upc: "6".to_string(),
            name: Some("Lamp".to_string()),
            price: Some(6999),
            weight: Some(300),
            notes: Some("Notes for lamp".to_string()),
            internal: Some("Internal for lamp".to_string()),
        },
        Product {
            upc: "7".to_string(),
            name: Some("Grill".to_string()),
            price: Some(3999),
            weight: Some(2000),
            notes: Some("Notes for grill".to_string()),
            internal: Some("Internal for grill".to_string()),
        },
        Product {
            upc: "8".to_string(),
            name: Some("Fridge".to_string()),
            price: Some(100000),
            weight: Some(6000),
            notes: Some("Notes for fridge".to_string()),
            internal: Some("Internal for fridge".to_string()),
        },
        Product {
            upc: "9".to_string(),
            name: Some("Sofa".to_string()),
            price: Some(9999),
            weight: Some(800),
            notes: Some("Notes for sofa".to_string()),
            internal: Some("Internal for sofa".to_string()),
        }
    ];
}
#[derive(SimpleObject, Clone)]
pub struct Product {
    upc: String,
    name: Option<String>,
    price: Option<i64>,
    weight: Option<i64>,
    notes: Option<String>,
    internal: Option<String>,
}

pub struct Query;

pub struct Mutation;

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

pub fn get_subgraph() -> Schema<Query, Mutation, EmptySubscription> {
    Schema::build(Query, Mutation, EmptySubscription)
        .enable_federation()
        .finish()
}

// Added for Multipart Example Plugin's E2E
#[Object(extends = true)]
impl Mutation {
    async fn upload(&self, ctx: &Context<'_>, file: Option<Upload>) -> String {
        if file.is_none() {
            return "No file uploaded".to_string();
        }
        // Write to a temp location, and return the path
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
