use jsonwebtoken::{encode, EncodingKey};
use sonic_rs::{json, JsonContainerTrait, JsonValueTrait, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::testkit::{
    coprocessor::TestCoprocessor, some_header_map, ClientResponseExt, TestRouter, TestSubgraphs,
};

const REQUIRED_POLICIES_KEY: &str = "hive::authorization::required_policies";

/// Router config wiring the `graphql.analysis` stage to the policy supergraph.
/// The stage receives the request context and answers with the granted policies.
fn policy_router_config(host: &str) -> String {
    format!(
        r#"
            supergraph:
              source: file
              path: supergraph-policy.graphql
            coprocessor:
              url: http://{host}/coprocessor
              protocol: http1
              stages:
                graphql:
                  analysis:
                    include:
                      context: true
            "#
    )
}

/// Same as [`policy_router_config`], with JWT authentication enabled so that
/// `@authenticated`/`@requiresScopes` are enforced alongside `@policy`.
fn policy_router_config_with_jwt(host: &str) -> String {
    format!(
        r#"
            supergraph:
              source: file
              path: supergraph-policy.graphql
            jwt:
              enabled: true
              require_authentication: false
              jwks_providers:
                - source: file
                  path: jwks.rsa512.json
            coprocessor:
              url: http://{host}/coprocessor
              protocol: http1
              stages:
                graphql:
                  analysis:
                    include:
                      context: true
            "#
    )
}

/// Builds a `graphql.analysis` answer that grants exactly the given policies by
/// echoing `required_policies` back with those entries set to `true`. Mirrors
/// how a coprocessor mutates Apollo Router's `apollo::authorization::required_policies`.
fn granting(policies: &[&str]) -> String {
    let granted: std::collections::HashMap<&str, bool> =
        policies.iter().map(|policy| (*policy, true)).collect();
    let decisions: Value = granted.iter().collect();

    json!({
        "version": 1,
        "control": "continue",
        "context": {
            REQUIRED_POLICIES_KEY: decisions,
        }
    })
    .to_string()
}

/// Reads the policy names published under `hive::authorization::required_policies`
/// out of a coprocessor payload, sorted so assertions do not depend on map ordering.
fn required_policies(payload: &Value) -> Option<Vec<String>> {
    let mut policies: Vec<String> = payload
        .get("context")?
        .pointer(&[REQUIRED_POLICIES_KEY])?
        .as_object()?
        .iter()
        .map(|(policy, _decision)| policy.to_string())
        .collect();

    policies.sort();
    Some(policies)
}

fn generate_jwt(payload: &Value) -> String {
    let pem = include_str!("../../jwks.rsa512.pem");

    encode::<Value>(
        &jsonwebtoken::Header {
            alg: jsonwebtoken::Algorithm::RS512,
            kid: Some("test_id".to_string()),
            ..Default::default()
        },
        payload,
        &EncodingKey::from_rsa_pem(pem.as_bytes()).expect("failed to read pem"),
    )
    .expect("failed to create token")
}

fn authorization_header() -> http::HeaderMap {
    some_header_map! {
        http::header::AUTHORIZATION => format!(
            "Bearer {}",
            generate_jwt(&json!({
                "sub": "user2",
                "iat": 1516239022,
                "exp": SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 3600,
            }))
        )
    }
    .unwrap()
}

/// The router publishes the policies the operation depends on so the coprocessor
/// knows what it has to decide on.
#[ntex::test]
async fn publishes_required_policies_to_the_coprocessor() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.analysis", |payload| {
            required_policies(payload).as_deref() == Some(&["read_inventory".to_string()])
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(granting(&["read_inventory"]))
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first: 1) { upc inStock } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "topProducts": [
          {
            "inStock": true,
            "upc": "1"
          }
        ]
      }
    }
    "#);

    analysis_stage_mock.assert_async().await;
}

/// Every policy of every OR group is published, deciding which combination is
/// enough is up to the router, not the coprocessor.
#[ntex::test]
async fn publishes_every_policy_of_an_or_group() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.analysis", |payload| {
            required_policies(payload).as_deref()
                == Some(&[
                    "admin".to_string(),
                    "internal".to_string(),
                    "read_users".to_string(),
                ])
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(granting(&["admin"]))
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ users { id } }", None, None)
        .await;

    assert!(
        response
            .json_body_string_pretty_stable()
            .await
            .contains("\"users\""),
        "the operation should have been executed"
    );

    analysis_stage_mock.assert_async().await;
}

/// A policy the coprocessor did not grant nulls the field it protects and reports
/// an error, exactly like the JWT-based authorization directives do.
#[ntex::test]
async fn filters_fields_whose_policy_was_not_granted() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage("graphql.analysis")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(granting(&[]))
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first: 1) { upc inStock } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "topProducts": [
          {
            "inStock": null,
            "upc": "1"
          }
        ]
      },
      "errors": [
        {
          "extensions": {
            "affectedPath": "topProducts.inStock",
            "code": "UNAUTHORIZED_FIELD_OR_TYPE"
          },
          "message": "Unauthorized field or type"
        }
      ]
    }
    "#);

    analysis_stage_mock.assert_async().await;
}

/// A coprocessor that leaves the granted policies out of its answer denies
/// everything, so the unresolved decision never defaults to "allowed".
#[ntex::test]
async fn denies_policies_left_undecided_by_the_coprocessor() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage("graphql.analysis")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"version": 1, "control": "continue"}).to_string())
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first: 1) { upc inStock } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "topProducts": [
          {
            "inStock": null,
            "upc": "1"
          }
        ]
      },
      "errors": [
        {
          "extensions": {
            "affectedPath": "topProducts.inStock",
            "code": "UNAUTHORIZED_FIELD_OR_TYPE"
          },
          "message": "Unauthorized field or type"
        }
      ]
    }
    "#);

    analysis_stage_mock.assert_async().await;
}

/// `required_policies` must stay an object of policy -> boolean/null, the shape a
/// coprocessor is expected to echo back. Anything else is rejected.
#[ntex::test]
async fn rejects_malformed_required_policies_from_the_coprocessor() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage("graphql.analysis")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "version": 1,
                "control": "continue",
                "context": {
                    REQUIRED_POLICIES_KEY: ["read_inventory"],
                }
            })
            .to_string(),
        )
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first: 1) { upc inStock } }", None, None)
        .await;

    assert!(
        !response.status().is_success(),
        "the router should reject a required_policies value that isn't an object"
    );

    analysis_stage_mock.assert_async().await;
}

/// Operations that touch no `@policy` field must not pay for a policy round trip:
/// nothing is published, and the coprocessor has nothing to decide.
#[ntex::test]
async fn publishes_no_policies_for_an_unprotected_operation() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage_with_matcher("graphql.analysis", |payload| {
            required_policies(payload).is_none()
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"version": 1, "control": "continue"}).to_string())
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request("{ topProducts(first: 1) { upc name } }", None, None)
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "topProducts": [
          {
            "name": "Table",
            "upc": "1"
          }
        ]
      }
    }
    "#);

    analysis_stage_mock.assert_async().await;
}

/// `@policy` and `@authenticated` on the same field are independent requirements,
/// granting the policy alone is not enough.
#[ntex::test]
async fn requires_authentication_on_top_of_the_granted_policy() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage("graphql.analysis")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(granting(&["read_weight"]))
        .expect(2)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config_with_jwt(&host))
        .build()
        .start()
        .await;

    let anonymous = router
        .send_graphql_request("{ topProducts(first: 1) { upc weight } }", None, None)
        .await;

    insta::assert_snapshot!(anonymous.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "topProducts": [
          {
            "upc": "1",
            "weight": null
          }
        ]
      },
      "errors": [
        {
          "extensions": {
            "affectedPath": "topProducts.weight",
            "code": "UNAUTHORIZED_FIELD_OR_TYPE"
          },
          "message": "Unauthorized field or type"
        }
      ]
    }
    "#);

    let authenticated = router
        .send_graphql_request(
            "{ topProducts(first: 1) { upc weight } }",
            None,
            Some(authorization_header()),
        )
        .await;

    insta::assert_snapshot!(authenticated.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "topProducts": [
          {
            "upc": "1",
            "weight": 100
          }
        ]
      }
    }
    "#);

    analysis_stage_mock.assert_async().await;
}

/// `interface SocialAccount.bio` defined with no policies in the subgraphs.
/// `type TwitterAccount.bio` is defined with `@policy(policies: [["read_bio"]])`.
/// `type GitHubAccount.bio` is defined with no policy.
///
/// The composed `interface SocialAccount.bio` is defined with a AND combo, so `@policy(policies: [["read_bio"]])`.
#[ntex::test]
async fn denies_interface_field_via_its_composed_policy_when_ungranted() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage("graphql.analysis")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(granting(&[]))
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config_with_jwt(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request(
            "{ me { socialAccounts { bio } } }",
            None,
            Some(authorization_header()),
        )
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "me": null
      },
      "errors": [
        {
          "extensions": {
            "affectedPath": "me.socialAccounts.bio",
            "code": "UNAUTHORIZED_FIELD_OR_TYPE"
          },
          "message": "Unauthorized field or type"
        }
      ]
    }
    "#);

    analysis_stage_mock.assert_async().await;
}

/// Same bare selection as above, but with `read_bio` granted: the interface
/// field's composed policy is satisfied, so it comes back for both concrete
/// types - including `GitHubAccount`, which never required anything itself.
#[ntex::test]
async fn allows_interface_field_on_every_implementor_once_the_policy_is_granted() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage("graphql.analysis")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(granting(&["read_bio"]))
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config_with_jwt(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request(
            "{ me { socialAccounts { bio } } }",
            None,
            Some(authorization_header()),
        )
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "me": {
          "socialAccounts": [
            {
              "bio": "Tweets about GraphQL"
            },
            {
              "bio": "Ships GraphQL routers"
            }
          ]
        }
      }
    }
    "#);

    analysis_stage_mock.assert_async().await;
}

/// Contrast with the two tests above: once the query discriminates by concrete
/// type via `... on` fragments, each implementor is authorized against its
/// *own* field-level policy instead of the interface's composed one.
#[ntex::test]
async fn fragment_scoped_interface_selection_only_denies_the_implementor_missing_its_policy() {
    let subgraphs = TestSubgraphs::builder().build().start().await;
    let mut coprocessor = TestCoprocessor::new().await;
    let host = coprocessor.host_with_port();

    let analysis_stage_mock = coprocessor
        .mock_stage("graphql.analysis")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(granting(&[]))
        .expect(1)
        .create();

    let router = TestRouter::builder()
        .with_subgraphs(&subgraphs)
        .inline_config(policy_router_config_with_jwt(&host))
        .build()
        .start()
        .await;

    let response = router
        .send_graphql_request(
            "{ me { socialAccounts { ... on TwitterAccount { bio } ... on GitHubAccount { bio } } } }",
            None,
            Some(authorization_header()),
        )
        .await;

    insta::assert_snapshot!(response.json_body_string_pretty_stable().await, @r#"
    {
      "data": {
        "me": null
      },
      "errors": [
        {
          "extensions": {
            "affectedPath": "me.socialAccounts.bio",
            "code": "UNAUTHORIZED_FIELD_OR_TYPE"
          },
          "message": "Unauthorized field or type"
        }
      ]
    }
    "#);

    analysis_stage_mock.assert_async().await;
}
