use async_graphql::{
    ComplexObject, EmptyMutation, EmptySubscription, Interface, Object, Schema, SimpleObject, ID,
};
use lazy_static::lazy_static;

lazy_static! {
    pub static ref USERS: Vec<User> = vec![
        User {
            id: ID("1".to_string()),
            name: Some("Uri Goldshtein".to_string()),
            username: Some("urigo".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("2".to_string()),
            name: Some("Dotan Simha".to_string()),
            username: Some("dotansimha".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("3".to_string()),
            name: Some("Kamil Kisiela".to_string()),
            username: Some("kamilkisiela".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("4".to_string()),
            name: Some("Arda Tanrikulu".to_string()),
            username: Some("ardatan".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("5".to_string()),
            name: Some("Gil Gardosh".to_string()),
            username: Some("gilgardosh".to_string()),
            birthday: Some(1234567890),
        },
        User {
            id: ID("6".to_string()),
            name: Some("Laurin Quast".to_string()),
            username: Some("laurin".to_string()),
            birthday: Some(1234567890),
        }
    ];
}

#[derive(Interface, Clone)]
#[allow(clippy::duplicated_attributes)] // async_graphql needs `ty` "duplicated"
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
    pub(crate) url: String,
    pub(crate) handle: String,
    pub(crate) followers: i32,
}

#[derive(SimpleObject, Clone)]
pub struct GitHubAccount {
    pub(crate) url: String,
    pub(crate) handle: String,
    pub(crate) repo_count: i32,
}

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct User {
    pub(crate) id: ID,
    pub(crate) name: Option<String>,
    pub(crate) username: Option<String>,
    pub(crate) birthday: Option<i32>,
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
}

impl User {
    fn me() -> User {
        USERS[0].clone()
    }
}

/// A non-null nested type whose only field errors. Used to verify that a `null`
/// from a non-null nested field bubbles up to the (non-null) parent.
pub struct NonNullNested;

#[Object]
impl NonNullNested {
    async fn field_that_errors(&self) -> async_graphql::Result<String> {
        Err(async_graphql::Error::new(
            "NonNullNested.fieldThatErrors always fails",
        ))
    }
}

/// A nullable nested type whose only field errors. Here the `null` stays on the
/// nullable field rather than bubbling up.
pub struct NullableNested;

#[Object]
impl NullableNested {
    async fn field_that_errors(&self, ctx: &async_graphql::Context<'_>) -> Option<String> {
        ctx.add_error(async_graphql::ServerError::new(
            "NullableNested.fieldThatErrors always fails",
            None,
        ));
        None
    }
}

pub struct Query;

#[Object(extends = true)]
impl Query {
    async fn me(&self) -> Option<User> {
        Some(User::me())
    }

    async fn user(&self, id: ID) -> Option<User> {
        USERS.iter().find(|user| user.id == id).cloned()
    }

    async fn users(&self) -> Option<Vec<Option<User>>> {
        Some(USERS.iter().map(|user| Some(user.clone())).collect())
    }

    /// A nullable root field whose resolver always errors. The subgraph reports
    /// the error and resolves the field to `null`.
    async fn nullable_field_that_errors(&self) -> async_graphql::Result<Option<String>> {
        Err(async_graphql::Error::new(
            "nullableFieldThatErrors always fails",
        ))
    }

    /// A non-null root field whose resolver always errors. Used to verify
    /// null-propagation for non-null root fields.
    async fn non_null_field_that_errors(&self) -> async_graphql::Result<String> {
        Err(async_graphql::Error::new(
            "nonNullFieldThatErrors always fails",
        ))
    }

    /// A non-null nested object; its inner field errors, so the `null` bubbles
    /// through `NonNullNested!` up to `data`.
    async fn non_null_nested(&self) -> NonNullNested {
        NonNullNested
    }

    /// A nullable nested object; its inner (nullable) field errors, so the `null`
    /// stays on that field.
    async fn nullable_nested(&self) -> Option<NullableNested> {
        Some(NullableNested)
    }

    #[graphql(entity)]
    async fn find_user_by_id(&self, id: ID) -> Option<User> {
        USERS.iter().find(|user| user.id == id).cloned()
    }
}

pub fn get_subgraph() -> Schema<Query, EmptyMutation, EmptySubscription> {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .enable_federation()
        .finish()
}
