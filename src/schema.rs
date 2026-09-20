use std::collections::BTreeMap;

use anyhow::{bail, Result};
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argument {
    pub name: String,
    /// Normalized type signature with all whitespace stripped, e.g.
    /// `String`, `String!`, `[ID!]!` — compared as text, not decomposed
    /// into list/nullability components (see README scope notes).
    pub type_sig: String,
    pub has_default: bool,
}

impl Argument {
    /// An argument is a real requirement on the caller only if it's
    /// non-null (`!`) AND has no default value to fall back to.
    pub fn is_required(&self) -> bool {
        self.type_sig.ends_with('!') && !self.has_default
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub type_sig: String,
    pub args: Vec<Argument>,
}

/// A field-container: `type`, `input`, or `interface`. All three share
/// the same `name: Type` field syntax, so they're pooled into one model
/// here rather than tracked as three separate kinds — a deliberate
/// simplification (see README).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectType {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumType {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub types: BTreeMap<String, ObjectType>,
    pub enums: BTreeMap<String, EnumType>,
}

/// Strips comments, descriptions, and directive usages so the structural
/// regexes below only ever see `keyword Name { ... }` shapes. Order
/// matters: block descriptions first (they can span lines and contain
/// `#`), then single-line quoted strings (covers both single-line
/// descriptions and `= "default"` argument values — only *presence* of
/// a default is tracked, never its content, so blanking the string is
/// harmless), then `#` line comments, then `@directive(...)` usages.
fn clean(input: &str) -> String {
    let block_desc = Regex::new(r#"(?s)"""(.*?)""""#).unwrap();
    let after_block = block_desc.replace_all(input, " ");

    let quoted = Regex::new(r#""[^"\n]*""#).unwrap();
    let after_quoted = quoted.replace_all(&after_block, " ");

    let line_comment = Regex::new(r"#[^\n]*").unwrap();
    let after_comment = line_comment.replace_all(&after_quoted, " ");

    let directive = Regex::new(r"@[A-Za-z_][A-Za-z0-9_]*\s*(\([^)]*\))?").unwrap();
    directive.replace_all(&after_comment, " ").into_owned()
}

/// Scans forward from just after an opening `{` and returns the index of
/// its matching `}`, tracking brace depth (real SDL bodies don't nest
/// braces, but this is no more expensive than assuming they don't and
/// is correct either way).
fn find_matching_brace(s: &str, start: usize) -> Result<usize> {
    let bytes = s.as_bytes();
    let mut depth = 1i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    bail!("unterminated block: missing closing '}}'")
}

fn parse_args(paren_str: &str) -> Vec<Argument> {
    let inner = paren_str
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    if inner.trim().is_empty() {
        return Vec::new();
    }
    // Top-level comma split. Doesn't account for commas nested inside a
    // list-literal default value (e.g. `= [1, 2, 3]`) — a documented
    // scope limit, same spirit as protodiff not resolving cross-file
    // imports.
    let arg_re = Regex::new(r"(\w+)\s*:\s*([\[\]!\w]+)(\s*=)?").unwrap();
    arg_re
        .captures_iter(inner)
        .map(|cap| {
            let name = cap[1].to_string();
            let type_sig: String = cap[2].chars().filter(|c| !c.is_whitespace()).collect();
            let has_default = cap.get(3).is_some();
            Argument {
                name,
                type_sig,
                has_default,
            }
        })
        .collect()
}

fn parse_fields(body: &str) -> Vec<Field> {
    let field_re = Regex::new(r"(\w+)\s*(\([^)]*\))?\s*:\s*([\[\]!\w]+)").unwrap();
    field_re
        .captures_iter(body)
        .map(|cap| {
            let name = cap[1].to_string();
            let type_sig: String = cap[3].chars().filter(|c| !c.is_whitespace()).collect();
            let args = cap
                .get(2)
                .map(|m| parse_args(m.as_str()))
                .unwrap_or_default();
            Field {
                name,
                type_sig,
                args,
            }
        })
        .collect()
}

fn parse_enum_values(body: &str) -> Vec<String> {
    let re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap();
    re.find_iter(body).map(|m| m.as_str().to_string()).collect()
}

/// Parses a GraphQL SDL document into a [`Schema`]. Deliberately not a
/// full spec-compliant parser (see README "Scope") — enough to diff
/// realistic `type`/`input`/`interface`/`enum` schemas for breaking
/// changes, not to validate arbitrary SDL.
pub fn parse_schema(input: &str) -> Result<Schema> {
    let cleaned = clean(input);
    let mut schema = Schema::default();

    let type_re =
        Regex::new(r"\b(type|input|interface)\s+(\w+)\s*(?:implements\s+[^{]*)?\{").unwrap();
    for cap in type_re.captures_iter(&cleaned) {
        let name = cap[2].to_string();
        let brace_start = cap.get(0).unwrap().end();
        let close = find_matching_brace(&cleaned, brace_start)?;
        let body = &cleaned[brace_start..close];
        let fields = parse_fields(body);
        schema
            .types
            .insert(name.clone(), ObjectType { name, fields });
    }

    let enum_re = Regex::new(r"\benum\s+(\w+)\s*\{").unwrap();
    for cap in enum_re.captures_iter(&cleaned) {
        let name = cap[1].to_string();
        let brace_start = cap.get(0).unwrap().end();
        let close = find_matching_brace(&cleaned, brace_start)?;
        let body = &cleaned[brace_start..close];
        let values = parse_enum_values(body);
        schema.enums.insert(name.clone(), EnumType { name, values });
    }

    if schema.types.is_empty() && schema.enums.is_empty() {
        bail!("no 'type', 'input', 'interface', or 'enum' declarations found — not a GraphQL SDL document?");
    }

    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_type_and_its_fields() {
        let schema = parse_schema("type User { id: ID! name: String }").unwrap();
        let user = &schema.types["User"];
        assert_eq!(user.fields.len(), 2);
        assert_eq!(user.fields[0].name, "id");
        assert_eq!(user.fields[0].type_sig, "ID!");
        assert_eq!(user.fields[1].name, "name");
        assert_eq!(user.fields[1].type_sig, "String");
    }

    #[test]
    fn parses_list_and_nested_nullability_type_signatures() {
        let schema = parse_schema("type Q { tags: [String!]! opt: [Int] }").unwrap();
        let q = &schema.types["Q"];
        assert_eq!(q.fields[0].type_sig, "[String!]!");
        assert_eq!(q.fields[1].type_sig, "[Int]");
    }

    #[test]
    fn parses_field_arguments_including_default_presence() {
        let schema =
            parse_schema("type Query { users(limit: Int = 10, active: Boolean!): [User] }")
                .unwrap();
        let field = &schema.types["Query"].fields[0];
        assert_eq!(field.args.len(), 2);
        assert_eq!(field.args[0].name, "limit");
        assert_eq!(field.args[0].type_sig, "Int");
        assert!(field.args[0].has_default);
        assert!(!field.args[0].is_required()); // has a default, so callers can omit it
        assert_eq!(field.args[1].name, "active");
        assert!(!field.args[1].has_default);
        assert!(field.args[1].is_required()); // non-null, no default
    }

    #[test]
    fn field_with_no_arguments_has_empty_args_list() {
        let schema = parse_schema("type User { id: ID! }").unwrap();
        assert!(schema.types["User"].fields[0].args.is_empty());
    }

    #[test]
    fn parses_enum_values() {
        let schema = parse_schema("enum Status { ACTIVE INACTIVE PENDING }").unwrap();
        assert_eq!(
            schema.enums["Status"].values,
            vec!["ACTIVE", "INACTIVE", "PENDING"]
        );
    }

    #[test]
    fn interface_and_input_are_parsed_as_field_containers_too() {
        let schema =
            parse_schema("interface Node { id: ID! }\ninput CreateUserInput { name: String! }")
                .unwrap();
        assert_eq!(schema.types["Node"].fields[0].name, "id");
        assert_eq!(schema.types["CreateUserInput"].fields[0].name, "name");
    }

    #[test]
    fn strips_block_descriptions_without_corrupting_the_following_field() {
        let schema = parse_schema(
            r#"
            """
            A user in the system.
            """
            type User {
                """The user's unique id"""
                id: ID!
                name: String
            }
            "#,
        )
        .unwrap();
        let user = &schema.types["User"];
        assert_eq!(user.fields.len(), 2);
        assert_eq!(user.fields[0].name, "id");
    }

    #[test]
    fn strips_line_comments() {
        let schema =
            parse_schema("type User {\n  id: ID! # the primary key\n  name: String\n}").unwrap();
        assert_eq!(schema.types["User"].fields.len(), 2);
    }

    #[test]
    fn strips_directive_usages() {
        let schema =
            parse_schema(r#"type User { oldField: String @deprecated(reason: "use newField") }"#)
                .unwrap();
        assert_eq!(schema.types["User"].fields[0].name, "oldField");
        assert_eq!(schema.types["User"].fields[0].type_sig, "String");
    }

    #[test]
    fn parses_multiple_declarations_packed_onto_a_single_line() {
        // A regression-shaped test in the spirit of protodiff's own
        // packed-single-line bug: this tool's structural regexes use
        // `\b` word boundaries rather than a line-start anchor for
        // exactly this reason.
        let schema =
            parse_schema("type A { id: ID! } type B { name: String } enum C { X Y }").unwrap();
        assert!(schema.types.contains_key("A"));
        assert!(schema.types.contains_key("B"));
        assert!(schema.enums.contains_key("C"));
    }

    #[test]
    fn empty_or_non_sdl_input_is_a_clean_error() {
        assert!(parse_schema("this is not a schema").is_err());
    }

    #[test]
    fn implements_clause_does_not_break_parsing() {
        let schema = parse_schema(
            "interface Node { id: ID! }\ntype User implements Node { id: ID! name: String }",
        )
        .unwrap();
        assert_eq!(schema.types["User"].fields.len(), 2);
    }
}
