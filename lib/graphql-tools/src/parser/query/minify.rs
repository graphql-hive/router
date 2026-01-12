use crate::parser::tokenizer::{Kind, Token, TokenStream};
use combine::StreamOnce;
use thiserror::Error;

/// Error minifying query
#[derive(Error, Debug)]
#[error("query minify error: {}", _0)]
pub struct MinifyError(String);

pub fn minify_query(source: &str) -> Result<String, MinifyError> {
    let mut bits: Vec<&str> = Vec::new();
    let mut stream = TokenStream::new(source);
    let mut prev_was_punctuator = false;

    loop {
        match stream.uncons() {
            Ok(x) => {
                let token: Token = x;
                let is_non_punctuator = token.kind != Kind::Punctuator;

                if prev_was_punctuator && is_non_punctuator {
                    bits.push(" ");
                }

                bits.push(token.value);
                prev_was_punctuator = is_non_punctuator;
            }
            Err(ref e) if e == &combine::easy::Error::end_of_input() => break,
            Err(e) => return Err(MinifyError(e.to_string())),
        }
    }

    Ok(bits.join(""))
}

#[cfg(test)]
mod tests {
    #[test]
    fn strip_ignored_characters() {
        let source = "
        query SomeQuery($foo: String!, $bar: String) {
            someField(foo: $foo, bar: $bar) {
                a
                b {
                    ... on B {
                        c
                        d
                    }
                }
            }
        }
        ";

        let minified =
            super::minify_query(source.to_string().as_str()).expect("minification failed");

        assert_eq!(
            &minified,
            "query SomeQuery($foo:String!$bar:String){someField(foo:$foo bar:$bar){a b{...on B{c d}}}}"
        );
    }

    #[test]
    fn unexpected_token() {
        let source = "
        query foo {
            bar;
        }
        ";

        let minified = super::minify_query(source.to_string().as_str());

        assert!(minified.is_err());

        assert_eq!(
            minified.unwrap_err().to_string(),
            "query minify error: Unexpected unexpected character ';'"
        );
    }

    #[test]
    fn test_minify_directive_args() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/directive_args.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query{node@dir(a:1 b:"2" c:true d:false e:null)}"#);
    }

    #[test]
    fn test_minify_directive_args_multiline() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/directive_args_multiline.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query{node@dir(a:1 b:"2" c:true d:false e:null)}"#);
    }

    #[test]
    fn test_minify_fragment() {
        let source = std::fs::read_to_string("src/parser/tests/queries/fragment.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"fragment frag on Friend{node}");
    }

    #[test]
    fn test_minify_fragment_spread() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/fragment_spread.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node{id...something}}");
    }

    #[test]
    fn test_minify_inline_fragment() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/inline_fragment.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node{id...on User{name}}}");
    }

    #[test]
    fn test_minify_inline_fragment_dir() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/inline_fragment_dir.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node{id...on User@defer{name}}}");
    }

    #[test]
    fn test_minify_kitchen_sink() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/kitchen-sink.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"
        query queryName($foo:ComplexType$site:Site=MOBILE){whoever123is:node(id:[123 456]){id...on User@defer{field2{id alias:field1(first:10 after:$foo)@include(if:$foo){id...frag}}}...@skip(unless:$foo){id}...{id}}}mutation likeStory{like(story:123)@defer{story{id}}}subscription StoryLikeSubscription($input:StoryLikeSubscribeInput){storyLikeSubscribe(input:$input){story{likers{count}likeSentence{text}}}}fragment frag on Friend{foo(size:$size bar:$b obj:{key:"value" block:"""

              block string uses \"""

          """})}{unnamed(truthy:true falsey:false nullish:null)query}
        "#);
    }

    #[test]
    fn test_minify_kitchen_sink_canonical() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/kitchen-sink_canonical.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query queryName($foo:ComplexType$site:Site=MOBILE){whoever123is:node(id:[123 456]){id...on User@defer{field2{id alias:field1(first:10 after:$foo)@include(if:$foo){id...frag}}}...@skip(unless:$foo){id}...{id}}}mutation likeStory{like(story:123)@defer{story{id}}}subscription StoryLikeSubscription($input:StoryLikeSubscribeInput){storyLikeSubscribe(input:$input){story{likers{count}likeSentence{text}}}}fragment frag on Friend{foo(size:$size bar:$b obj:{block:"block string uses \"\"\"" key:"value"})}{unnamed(truthy:true falsey:false nullish:null)query}"#);
    }

    #[test]
    fn test_minify_minimal() {
        let source = std::fs::read_to_string("src/parser/tests/queries/minimal.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"{a}");
    }

    #[test]
    fn test_minify_minimal_mutation() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/minimal_mutation.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"mutation{notify}");
    }

    #[test]
    fn test_minify_minimal_query() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/minimal_query.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node}");
    }

    #[test]
    fn test_minify_mutation_directive() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/mutation_directive.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"mutation@directive{node}");
    }

    #[test]
    fn test_minify_mutation_nameless_vars() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/mutation_nameless_vars.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"mutation($first:Int$second:Int){field1(first:$first)field2(second:$second)}");
    }

    #[test]
    fn test_minify_named_query() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/named_query.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo{field}");
    }

    #[test]
    fn test_minify_nested_selection() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/nested_selection.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node{id}}");
    }

    #[test]
    fn test_minify_query_aliases() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_aliases.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{an_alias:node}");
    }

    #[test]
    fn test_minify_query_arguments() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_arguments.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1)}");
    }

    #[test]
    fn test_minify_query_arguments_multiline() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_arguments_multiline.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1)node(id:1 one:3)}");
    }

    #[test]
    fn test_minify_query_array_argument_multiline() {
        let source = std::fs::read_to_string(
            "src/parser/tests/queries/query_array_argument_multiline.graphql",
        )
        .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:[5 6 7])}");
    }

    #[test]
    fn test_minify_query_directive() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_directive.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query@directive{node}");
    }

    #[test]
    fn test_minify_query_list_argument() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_list_argument.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1 list:[123 456])}");
    }

    #[test]
    fn test_minify_query_nameless_vars() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_nameless_vars.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query($first:Int$second:Int){field1(first:$first)field2(second:$second)}");
    }

    #[test]
    fn test_minify_query_nameless_vars_multiple_fields() {
        let source = std::fs::read_to_string(
            "src/parser/tests/queries/query_nameless_vars_multiple_fields.graphql",
        )
        .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query($houseId:String!$streetNumber:Int!){house(id:$houseId){id name lat lng}street(number:$streetNumber){id}houseStreet(id:$houseId number:$streetNumber){id}}");
    }

    #[test]
    fn test_minify_query_nameless_vars_multiple_fields_canonical() {
        let source = std::fs::read_to_string(
            "src/parser/tests/queries/query_nameless_vars_multiple_fields_canonical.graphql",
        )
        .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query($houseId:String!$streetNumber:Int!){house(id:$houseId){id name lat lng}street(number:$streetNumber){id}houseStreet(id:$houseId number:$streetNumber){id}}");
    }

    #[test]
    fn test_minify_query_object_argument() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_object_argument.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1 obj:{key1:123 key2:456})}");
    }

    #[test]
    fn test_minify_query_object_argument_multiline() {
        let source = std::fs::read_to_string(
            "src/parser/tests/queries/query_object_argument_multiline.graphql",
        )
        .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1 obj:{key1:123 key2:456})}");
    }

    #[test]
    fn test_minify_query_var_default_float() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_default_float.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($site:Float=0.5){field}");
    }

    #[test]
    fn test_minify_query_var_default_list() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_default_list.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($site:[Int]=[123 456]){field}");
    }

    #[test]
    fn test_minify_query_var_default_object() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_default_object.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($site:Site={url:null}){field}");
    }

    #[test]
    fn test_minify_query_var_default_string() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_default_string.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query Foo($site:String="string"){field}"#);
    }

    #[test]
    fn test_minify_query_var_defaults() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_defaults.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($site:Site=MOBILE){field}");
    }

    #[test]
    fn test_minify_query_vars() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_vars.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($arg:SomeType){field}");
    }

    #[test]
    fn test_minify_string_literal() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/string_literal.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query{node(id:"hello")}"#);
    }

    #[test]
    fn test_minify_subscription_directive() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/subscription_directive.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"subscription@directive{node}");
    }

    #[test]
    fn test_minify_triple_quoted_literal() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/triple_quoted_literal.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"
        query{node(id:"""
            Hello,
              world!
          """)}
        "#);
    }
}
