use combine::easy::{Error, Errors, Info};
use combine::stream::ResetStream;
use combine::{Positioned, StreamOnce};

use super::ast::*;
use super::error::{InternalError, ParseError};
use crate::parser::common::{
    unquote_block_string, unquote_string, Directive, Number, Text, Type, Value,
};
use crate::parser::position::Pos;
use crate::parser::tokenizer::{Kind, Token, TokenStream};

type PResult<'a, T> = Result<T, InternalError<'a>>;

struct Cursor<'a> {
    stream: TokenStream<'a>,
}

impl<'a> Cursor<'a> {
    fn new(stream: TokenStream<'a>) -> Self {
        Self { stream }
    }

    fn pos(&self) -> Pos {
        self.stream.position()
    }

    fn offset(&self) -> usize {
        self.stream.offset()
    }

    fn next(&mut self) -> PResult<'a, Token<'a>> {
        let pos = self.stream.position();
        self.stream.uncons().map_err(|e| error_at(pos, e))
    }

    fn peek(&mut self) -> PResult<'a, Token<'a>> {
        let pos = self.pos();
        let cp = self.stream.checkpoint();
        match self.stream.uncons() {
            Ok(tok) => {
                self.stream.reset(cp).unwrap();
                Ok(tok)
            }
            Err(e) => Err(error_at(pos, e)),
        }
    }
}

#[inline]
fn error_at<'a>(pos: Pos, err: Error<Token<'a>, Token<'a>>) -> InternalError<'a> {
    Errors::new(pos, err)
}

#[inline]
fn unexpected_err<'a>(pos: Pos, tok: Token<'a>) -> InternalError<'a> {
    Errors::new(pos, Error::Unexpected(Info::Token(tok)))
}

#[inline]
fn unexpected_with_expected<'a>(pos: Pos, tok: Token<'a>, what: &'static str) -> InternalError<'a> {
    let mut errors = Errors::new(pos, Error::Unexpected(Info::Token(tok)));
    errors.add_error(Error::Expected(Info::Static(what)));
    errors
}

fn parse_float<'a>(value: &str) -> PResult<'a, f64> {
    value.parse::<f64>().map_err(|_| {
        Errors::new(
            Pos::default(),
            Error::Unexpected(Info::Owned(format!("unsupported float {:?}", value))),
        )
    })
}

fn value<'a, const ALLOW_VARIABLES: bool, S: Text<'a>>(
    c: &mut Cursor<'a>,
) -> PResult<'a, Value<'a, S>> {
    let tok = c.peek()?;
    match tok.kind {
        Kind::Name => match tok.value {
            "true" => {
                c.next()?;
                Ok(Value::Boolean(true))
            }
            "false" => {
                c.next()?;
                Ok(Value::Boolean(false))
            }
            "null" => {
                c.next()?;
                Ok(Value::Null)
            }
            _ => {
                let _ = c.next();
                Ok(Value::Enum(S::Value::from(tok.value)))
            }
        },
        Kind::IntValue => {
            let num = tok.value.parse::<i64>().map_err(|_| {
                Errors::new(
                    c.pos(),
                    Error::Unexpected(Info::Owned(format!("unsupported integer {:?}", tok.value))),
                )
            })?;
            c.next()?;
            Ok(Value::Int(Number(num)))
        }
        Kind::FloatValue => {
            let val = parse_float(tok.value)?;
            c.next()?;
            Ok(Value::Float(val))
        }
        Kind::StringValue => {
            let pos = c.pos();
            let raw = tok.value;
            let s = unquote_string(raw).map_err(|err| Errors::new(pos, err))?;
            c.next()?;
            Ok(Value::String(s))
        }
        Kind::BlockString => {
            let pos = c.pos();
            let raw = tok.value;
            let s = unquote_block_string(raw).map_err(|err| Errors::new(pos, err))?;
            c.next()?;
            Ok(Value::String(s))
        }
        Kind::Punctuator => match tok.value {
            "$" => {
                if !ALLOW_VARIABLES {
                    return Err(unexpected_with_expected(c.pos(), tok, "default value"));
                }
                c.next()?;
                let name_tok = bump_kind(c, Kind::Name, "Name")?;
                Ok(Value::Variable(S::Value::from(name_tok.value)))
            }
            "[" => {
                c.next()?;
                let mut items = Vec::new();
                loop {
                    let next = c.peek()?;
                    if next.kind == Kind::Punctuator && next.value == "]" {
                        c.next()?;
                        break;
                    }
                    items.push(value::<ALLOW_VARIABLES, S>(c)?);
                }
                Ok(Value::List(items))
            }
            "{" => {
                c.next()?;
                let mut items = Vec::new();
                loop {
                    let next = c.peek()?;
                    if next.kind == Kind::Punctuator && next.value == "}" {
                        c.next()?;
                        break;
                    }
                    let name_tok = bump_kind(c, Kind::Name, "Name")?;
                    bump_punct(c, ":")?;
                    let val = value::<ALLOW_VARIABLES, S>(c)?;
                    items.push((S::Value::from(name_tok.value), val));
                }
                Ok(Value::Object(items))
            }
            _ => {
                let pos = c.pos();
                let _ = c.next();
                Err(unexpected_err(pos, tok))
            }
        },
    }
}

fn parse_type<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, Type<'a, S>> {
    let tok = c.peek()?;
    let typ = match tok.kind {
        Kind::Punctuator if tok.value == "[" => {
            c.next()?;
            let inner = parse_type::<S>(c)?;
            bump_punct(c, "]")?;
            Type::ListType(Box::new(inner))
        }
        Kind::Name => {
            let name_tok = c.next()?;
            Type::NamedType(S::Value::from(name_tok.value))
        }
        _ => {
            let pos = c.pos();
            let tok = c.next()?;
            return Err(unexpected_with_expected(pos, tok, "type"));
        }
    };
    let next = c.peek()?;
    if next.kind == Kind::Punctuator && next.value == "!" {
        c.next()?;
        Ok(Type::NonNullType(Box::new(typ)))
    } else {
        Ok(typ)
    }
}

fn arguments<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, Vec<(S::Value, Value<'a, S>)>> {
    let next = c.peek()?;
    if next.kind != Kind::Punctuator || next.value != "(" {
        return Ok(Vec::new());
    }
    c.next()?;
    if is_punct(c.peek()?, ")") {
        let pos = c.pos();
        let tok = c.next()?;
        return Err(unexpected_with_expected(pos, tok, "Name"));
    }
    let mut args = Vec::new();
    loop {
        let next = c.peek()?;
        if next.kind == Kind::Punctuator && next.value == ")" {
            c.next()?;
            break;
        }
        let name_tok = bump_kind(c, Kind::Name, "Name")?;
        bump_punct(c, ":")?;
        let val = value::<true, S>(c)?;
        args.push((S::Value::from(name_tok.value), val));
    }
    Ok(args)
}

fn directives<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, Vec<Directive<'a, S>>> {
    let mut dirs = Vec::new();
    loop {
        let next = c.peek()?;
        if next.kind != Kind::Punctuator || next.value != "@" {
            break;
        }
        let pos = c.pos();
        c.next()?;
        let name_tok = bump_kind(c, Kind::Name, "Name")?;
        let args = arguments::<S>(c)?;
        dirs.push(Directive {
            position: pos,
            name: S::Value::from(name_tok.value),
            arguments: args,
        });
    }
    Ok(dirs)
}

fn selection_set<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, SelectionSet<'a, S>> {
    let start = c.pos();
    bump_punct(c, "{")?;
    if is_punct(c.peek()?, "}") {
        let pos = c.pos();
        let tok = c.next()?;
        return Err(unexpected_with_expected(pos, tok, "Name or ..."));
    }
    let mut items = Vec::new();
    loop {
        let next = c.peek()?;
        if next.kind == Kind::Punctuator && next.value == "}" {
            c.next()?;
            break;
        }
        items.push(selection::<S>(c)?);
    }
    let end = c.pos();
    Ok(SelectionSet {
        span: (start, end),
        items,
    })
}

fn selection<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, Selection<'a, S>> {
    let tok = c.peek()?;
    match tok.kind {
        Kind::Name => field::<S>(c).map(Selection::Field),
        Kind::Punctuator if tok.value == "..." => {
            c.next()?;
            let next = c.peek()?;
            match next.kind {
                Kind::Punctuator if next.value == "@" || next.value == "{" => {
                    inline_fragment_after_ellipsis::<S>(c, c.pos())
                }
                Kind::Name if next.value == "on" => inline_fragment_after_ellipsis::<S>(c, c.pos()),
                Kind::Name => fragment_spread_after_ellipsis::<S>(c, c.pos()),
                _ => fragment_spread_after_ellipsis::<S>(c, c.pos()),
            }
        }
        _ => {
            let pos = c.pos();
            let tok = c.next()?;
            Err(unexpected_with_expected(pos, tok, "Name or ..."))
        }
    }
}

fn inline_fragment_after_ellipsis<'a, S: Text<'a>>(
    c: &mut Cursor<'a>,
    pos: Pos,
) -> PResult<'a, Selection<'a, S>> {
    let type_condition = match c.peek()? {
        Token {
            kind: Kind::Name,
            value: "on",
        } => {
            c.next()?;
            let name = bump_name::<S>(c)?;
            Some(TypeCondition::On(name))
        }
        _ => None,
    };
    let dirs = directives::<S>(c)?;
    let sel_set = selection_set::<S>(c)?;
    Ok(Selection::InlineFragment(InlineFragment {
        position: pos,
        type_condition,
        directives: dirs,
        selection_set: sel_set,
    }))
}

fn fragment_spread_after_ellipsis<'a, S: Text<'a>>(
    c: &mut Cursor<'a>,
    pos: Pos,
) -> PResult<'a, Selection<'a, S>> {
    let name = bump_name::<S>(c)?;
    let dirs = directives::<S>(c)?;
    Ok(Selection::FragmentSpread(FragmentSpread {
        position: pos,
        fragment_name: name,
        directives: dirs,
    }))
}

fn field<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, Field<'a, S>> {
    let pos = c.pos();
    let first = bump_name::<S>(c)?;
    let next = c.peek()?;
    let (alias, name) = if next.kind == Kind::Punctuator && next.value == ":" {
        c.next()?;
        let second = bump_name::<S>(c)?;
        (Some(first), second)
    } else {
        (None, first)
    };
    let args = arguments::<S>(c)?;
    let dirs = directives::<S>(c)?;
    let sel_set = match c.peek()? {
        Token {
            kind: Kind::Punctuator,
            value: "{",
        } => selection_set::<S>(c)?,
        _ => SelectionSet {
            span: (pos, pos),
            items: Vec::new(),
        },
    };
    Ok(Field {
        position: pos,
        alias,
        name,
        arguments: args,
        directives: dirs,
        selection_set: sel_set,
    })
}

fn variable_definition<'a, S: Text<'a>>(
    c: &mut Cursor<'a>,
) -> PResult<'a, VariableDefinition<'a, S>> {
    let pos = c.pos();
    bump_punct(c, "$")?;
    let name_tok = bump_kind(c, Kind::Name, "Name")?;
    bump_punct(c, ":")?;
    let var_type = parse_type::<S>(c)?;
    let next = c.peek()?;
    let default_val = if next.kind == Kind::Punctuator && next.value == "=" {
        c.next()?;
        Some(value::<false, S>(c)?)
    } else {
        None
    };
    Ok(VariableDefinition {
        position: pos,
        name: S::Value::from(name_tok.value),
        var_type,
        default_value: default_val,
    })
}

fn variable_definitions<'a, S: Text<'a>>(
    c: &mut Cursor<'a>,
) -> PResult<'a, Vec<VariableDefinition<'a, S>>> {
    let next = c.peek()?;
    if next.kind != Kind::Punctuator || next.value != "(" {
        return Ok(Vec::new());
    }
    c.next()?;
    if is_punct(c.peek()?, ")") {
        let pos = c.pos();
        let tok = c.next()?;
        return Err(unexpected_with_expected(pos, tok, "$"));
    }
    let mut vars = Vec::new();
    loop {
        let next = c.peek()?;
        if next.kind == Kind::Punctuator && next.value == ")" {
            c.next()?;
            break;
        }
        vars.push(variable_definition::<S>(c)?);
    }
    Ok(vars)
}

fn operation_definition<'a, S: Text<'a>>(
    c: &mut Cursor<'a>,
) -> PResult<'a, OperationDefinition<'a, S>> {
    let tok = c.peek()?;
    match tok.kind {
        Kind::Punctuator if tok.value == "{" => {
            Ok(OperationDefinition::SelectionSet(selection_set::<S>(c)?))
        }
        Kind::Name => match tok.value {
            "query" => {
                let pos = c.pos();
                c.next()?;
                let (name, vars, dirs, sel_set) = operation_parts::<S>(c)?;
                Ok(OperationDefinition::Query(Query {
                    position: pos,
                    name,
                    variable_definitions: vars,
                    directives: dirs,
                    selection_set: sel_set,
                }))
            }
            "mutation" => {
                let pos = c.pos();
                c.next()?;
                let (name, vars, dirs, sel_set) = operation_parts::<S>(c)?;
                Ok(OperationDefinition::Mutation(Mutation {
                    position: pos,
                    name,
                    variable_definitions: vars,
                    directives: dirs,
                    selection_set: sel_set,
                }))
            }
            "subscription" => {
                let pos = c.pos();
                c.next()?;
                let (name, vars, dirs, sel_set) = operation_parts::<S>(c)?;
                Ok(OperationDefinition::Subscription(Subscription {
                    position: pos,
                    name,
                    variable_definitions: vars,
                    directives: dirs,
                    selection_set: sel_set,
                }))
            }
            _ => {
                let pos = c.pos();
                let tok = c.next()?;
                Err(unexpected_with_expected(
                    pos,
                    tok,
                    "{, query, mutation, subscription or fragment",
                ))
            }
        },
        _ => {
            let pos = c.pos();
            let tok = c.next()?;
            Err(unexpected_with_expected(
                pos,
                tok,
                "{, query, mutation, subscription or fragment",
            ))
        }
    }
}

fn operation_parts<'a, S: Text<'a>>(
    c: &mut Cursor<'a>,
) -> PResult<
    'a,
    (
        Option<S::Value>,
        Vec<VariableDefinition<'a, S>>,
        Vec<Directive<'a, S>>,
        SelectionSet<'a, S>,
    ),
> {
    let name = try_operation_name::<S>(c)?;
    let vars = variable_definitions::<S>(c)?;
    let dirs = directives::<S>(c)?;
    let selection_set = selection_set::<S>(c)?;
    Ok((name, vars, dirs, selection_set))
}

fn try_operation_name<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, Option<S::Value>> {
    let next = c.peek()?;
    if next.kind == Kind::Name {
        let name_tok = c.next()?;
        Ok(Some(S::Value::from(name_tok.value)))
    } else {
        Ok(None)
    }
}

fn fragment_definition<'a, S: Text<'a>>(
    c: &mut Cursor<'a>,
) -> PResult<'a, FragmentDefinition<'a, S>> {
    let pos = c.pos();
    bump_ident(c, "fragment")?;
    let name = bump_name::<S>(c)?;
    bump_ident(c, "on")?;
    let type_cond_name = bump_name::<S>(c)?;
    let dirs = directives::<S>(c)?;
    let sel_set = selection_set::<S>(c)?;
    Ok(FragmentDefinition {
        position: pos,
        name,
        type_condition: TypeCondition::On(type_cond_name),
        directives: dirs,
        selection_set: sel_set,
    })
}

fn definition<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, Definition<'a, S>> {
    let tok = c.peek()?;
    match tok.kind {
        Kind::Punctuator if tok.value == "{" => {
            operation_definition::<S>(c).map(Definition::Operation)
        }
        Kind::Name => match tok.value {
            "query" | "mutation" | "subscription" => {
                operation_definition::<S>(c).map(Definition::Operation)
            }
            "fragment" => fragment_definition::<S>(c).map(Definition::Fragment),
            _ => {
                let pos = c.pos();
                let tok = c.next()?;
                Err(unexpected_with_expected(
                    pos,
                    tok,
                    "{, query, mutation, subscription or fragment",
                ))
            }
        },
        _ => {
            let pos = c.pos();
            let tok = c.next()?;
            Err(unexpected_with_expected(
                pos,
                tok,
                "{, query, mutation, subscription or fragment",
            ))
        }
    }
}

fn document<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, Document<'a, S>> {
    let mut defs = Vec::new();
    loop {
        let next = c.peek();
        match next {
            Ok(tok) if tok.kind == Kind::Punctuator && tok.value == "}" => {
                return Err(unexpected_err(c.pos(), tok));
            }
            Err(ref e) if is_end_of_input_or_limit(e) => break,
            Err(e) => return Err(e),
            _ => {}
        }
        defs.push(definition::<S>(c)?);
    }
    if defs.is_empty() {
        let pos = c.pos();
        let tok = c.next();
        match tok {
            Ok(tok) => return Err(unexpected_err(pos, tok)),
            Err(e) => return Err(e),
        }
    }
    Ok(Document { definitions: defs })
}

fn is_end_of_input_or_limit<'a>(err: &InternalError<'a>) -> bool {
    err.errors.iter().any(|e| match e {
        Error::Unexpected(Info::Static("end of input")) => true,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_query, parse_query_with_token_limit};
    use crate::parser::query::{Definition, OperationDefinition, Selection, Value};

    #[test]
    fn rejects_empty_required_constructs() {
        assert!(parse_query::<String>("{}").is_err());
        assert!(parse_query::<String>("{ field() }").is_err());
        assert!(parse_query::<String>("query Example() { field }").is_err());
    }

    #[test]
    fn rejects_variables_in_nested_defaults() {
        assert!(parse_query::<String>("query Example($x: Int = [$y]) { field }").is_err());
        assert!(parse_query::<String>("query Example($x: Int = { value: $y }) { field }").is_err());
    }

    #[test]
    fn parses_escaped_strings() {
        let document = parse_query::<String>(r#"{ field(value: "line\n\b") }"#).unwrap();
        let Definition::Operation(OperationDefinition::SelectionSet(selection_set)) =
            &document.definitions[0]
        else {
            panic!("expected a selection set");
        };
        let Selection::Field(field) = &selection_set.items[0] else {
            panic!("expected a field");
        };
        assert_eq!(
            field.arguments[0].1,
            Value::String("line\n\u{0008}".to_string())
        );
    }

    #[test]
    fn token_limit_is_an_error() {
        assert!(parse_query_with_token_limit::<String>(
            "query First { first } query Second { second }",
            5,
        )
        .is_err());
    }
}

fn bump_name<'a, S: Text<'a>>(c: &mut Cursor<'a>) -> PResult<'a, S::Value> {
    let tok = bump_kind(c, Kind::Name, "Name")?;
    Ok(S::Value::from(tok.value))
}

#[inline]
fn is_punct(tok: Token<'_>, value: &str) -> bool {
    tok.kind == Kind::Punctuator && tok.value == value
}

fn bump_kind<'a>(
    c: &mut Cursor<'a>,
    expected: Kind,
    label: &'static str,
) -> PResult<'a, Token<'a>> {
    let pos = c.pos();
    let tok = c.next()?;
    if tok.kind == expected {
        Ok(tok)
    } else {
        Err(unexpected_with_expected(pos, tok, label))
    }
}

fn bump_punct<'a>(c: &mut Cursor<'a>, value: &'static str) -> PResult<'a, ()> {
    let pos = c.pos();
    let tok = c.next()?;
    if tok.kind == Kind::Punctuator && tok.value == value {
        Ok(())
    } else {
        Err(unexpected_with_expected(pos, tok, value))
    }
}

fn bump_ident<'a>(c: &mut Cursor<'a>, value: &'static str) -> PResult<'a, ()> {
    let pos = c.pos();
    let tok = c.next()?;
    if tok.kind == Kind::Name && tok.value == value {
        Ok(())
    } else {
        Err(unexpected_with_expected(pos, tok, value))
    }
}

pub fn parse_query<'a, S>(s: &'a str) -> Result<Document<'a, S>, ParseError>
where
    S: Text<'a>,
{
    let tokens = TokenStream::new(s);
    parse_query_impl(tokens)
}

pub fn parse_query_with_token_limit<'a, S>(
    s: &'a str,
    token_limit: usize,
) -> Result<Document<'a, S>, ParseError>
where
    S: Text<'a>,
{
    let tokens = TokenStream::new_with_token_limit(s, token_limit);
    parse_query_impl(tokens)
}

fn parse_query_impl<'a, S>(tokens: TokenStream<'a>) -> Result<Document<'a, S>, ParseError>
where
    S: Text<'a>,
{
    let mut c = Cursor::new(tokens);
    document::<S>(&mut c).map_err(ParseError::from)
}

pub fn consume_definition<'a, S>(s: &'a str) -> Result<(Definition<'a, S>, &'a str), ParseError>
where
    S: Text<'a>,
{
    let tokens = TokenStream::new(s);
    let mut c = Cursor::new(tokens);
    let def = definition::<S>(&mut c).map_err(ParseError::from)?;
    let remainder = &s[c.offset()..];
    Ok((def, remainder))
}
