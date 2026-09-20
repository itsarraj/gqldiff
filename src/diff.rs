use crate::schema::Schema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    RemovedType {
        name: String,
    },
    RemovedEnum {
        name: String,
    },
    RemovedField {
        type_name: String,
        field: String,
    },
    RemovedEnumValue {
        enum_name: String,
        value: String,
    },
    FieldTypeChanged {
        type_name: String,
        field: String,
        old_type: String,
        new_type: String,
    },
    NewRequiredArgument {
        type_name: String,
        field: String,
        arg: String,
    },
    ArgumentBecameRequired {
        type_name: String,
        field: String,
        arg: String,
    },
}

impl Change {
    pub fn describe(&self) -> String {
        match self {
            Change::RemovedType { name } => format!("type '{name}' removed"),
            Change::RemovedEnum { name } => format!("enum '{name}' removed"),
            Change::RemovedField { type_name, field } => {
                format!("{type_name}.{field} removed")
            }
            Change::RemovedEnumValue { enum_name, value } => {
                format!("{enum_name}: enum value '{value}' removed")
            }
            Change::FieldTypeChanged {
                type_name,
                field,
                old_type,
                new_type,
            } => {
                format!("{type_name}.{field}: type changed from {old_type} to {new_type}")
            }
            Change::NewRequiredArgument {
                type_name,
                field,
                arg,
            } => {
                format!(
                    "{type_name}.{field}: new required argument '{arg}' — existing callers don't send it"
                )
            }
            Change::ArgumentBecameRequired {
                type_name,
                field,
                arg,
            } => {
                format!("{type_name}.{field}: argument '{arg}' was optional, is now required")
            }
        }
    }
}

/// Compares `old` against `new`, reporting only changes that can break
/// an existing client compiled/generated against the old schema — not
/// every difference. A new type, a new enum, a new optional field, a
/// new value on a still-present enum, and a new *optional* argument are
/// all additive and deliberately produce no [`Change`].
pub fn diff_schemas(old: &Schema, new: &Schema) -> Vec<Change> {
    let mut changes = Vec::new();

    for (name, old_type) in &old.types {
        let Some(new_type) = new.types.get(name) else {
            changes.push(Change::RemovedType { name: name.clone() });
            continue;
        };

        for old_field in &old_type.fields {
            let Some(new_field) = new_type.fields.iter().find(|f| f.name == old_field.name) else {
                changes.push(Change::RemovedField {
                    type_name: name.clone(),
                    field: old_field.name.clone(),
                });
                continue;
            };

            if old_field.type_sig != new_field.type_sig {
                changes.push(Change::FieldTypeChanged {
                    type_name: name.clone(),
                    field: old_field.name.clone(),
                    old_type: old_field.type_sig.clone(),
                    new_type: new_field.type_sig.clone(),
                });
            }

            for new_arg in &new_field.args {
                match old_field.args.iter().find(|a| a.name == new_arg.name) {
                    None => {
                        if new_arg.is_required() {
                            changes.push(Change::NewRequiredArgument {
                                type_name: name.clone(),
                                field: old_field.name.clone(),
                                arg: new_arg.name.clone(),
                            });
                        }
                    }
                    Some(old_arg) => {
                        if !old_arg.is_required() && new_arg.is_required() {
                            changes.push(Change::ArgumentBecameRequired {
                                type_name: name.clone(),
                                field: old_field.name.clone(),
                                arg: new_arg.name.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    for (name, old_enum) in &old.enums {
        let Some(new_enum) = new.enums.get(name) else {
            changes.push(Change::RemovedEnum { name: name.clone() });
            continue;
        };
        for value in &old_enum.values {
            if !new_enum.values.contains(value) {
                changes.push(Change::RemovedEnumValue {
                    enum_name: name.clone(),
                    value: value.clone(),
                });
            }
        }
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::parse_schema;

    fn diff(old: &str, new: &str) -> Vec<Change> {
        let old = parse_schema(old).unwrap();
        let new = parse_schema(new).unwrap();
        diff_schemas(&old, &new)
    }

    #[test]
    fn removed_field_is_breaking() {
        let changes = diff(
            "type User { id: ID! name: String }",
            "type User { id: ID! }",
        );
        assert_eq!(
            changes,
            vec![Change::RemovedField {
                type_name: "User".into(),
                field: "name".into()
            }]
        );
    }

    #[test]
    fn removed_type_is_breaking() {
        let changes = diff(
            "type User { id: ID! } type Ghost { x: Int }",
            "type User { id: ID! }",
        );
        assert_eq!(
            changes,
            vec![Change::RemovedType {
                name: "Ghost".into()
            }]
        );
    }

    #[test]
    fn new_type_is_not_breaking() {
        let changes = diff(
            "type User { id: ID! }",
            "type User { id: ID! } type Post { id: ID! }",
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn new_optional_field_is_not_breaking() {
        let changes = diff(
            "type User { id: ID! }",
            "type User { id: ID! nickname: String }",
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn field_type_changed_scalar_is_breaking() {
        let changes = diff("type User { age: String }", "type User { age: Int }");
        assert_eq!(
            changes,
            vec![Change::FieldTypeChanged {
                type_name: "User".into(),
                field: "age".into(),
                old_type: "String".into(),
                new_type: "Int".into(),
            }]
        );
    }

    #[test]
    fn nullable_to_non_null_is_breaking() {
        let changes = diff("type User { name: String }", "type User { name: String! }");
        assert_eq!(
            changes,
            vec![Change::FieldTypeChanged {
                type_name: "User".into(),
                field: "name".into(),
                old_type: "String".into(),
                new_type: "String!".into(),
            }]
        );
    }

    #[test]
    fn non_null_to_nullable_is_still_reported_as_a_type_change() {
        // Loosening nullability is arguably safer than tightening it,
        // but it's still a wire/type-shape change existing strongly-typed
        // codegen clients (this tool's real audience) can choke on, so
        // it's reported the same as any other type-signature change
        // rather than special-cased as silently safe.
        let changes = diff("type User { name: String! }", "type User { name: String }");
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0], Change::FieldTypeChanged { .. }));
    }

    #[test]
    fn new_required_argument_is_breaking() {
        let changes = diff(
            "type Query { users: [User] }",
            "type Query { users(active: Boolean!): [User] }",
        );
        assert_eq!(
            changes,
            vec![Change::NewRequiredArgument {
                type_name: "Query".into(),
                field: "users".into(),
                arg: "active".into(),
            }]
        );
    }

    #[test]
    fn new_optional_argument_is_not_breaking() {
        let changes = diff(
            "type Query { users: [User] }",
            "type Query { users(limit: Int): [User] }",
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn new_required_argument_with_default_is_not_breaking() {
        // Non-null type but a default value means existing callers who
        // omit it still get a value — not a real requirement.
        let changes = diff(
            "type Query { users: [User] }",
            "type Query { users(limit: Int! = 10): [User] }",
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn existing_argument_becoming_required_is_breaking() {
        let changes = diff(
            "type Query { users(limit: Int): [User] }",
            "type Query { users(limit: Int!): [User] }",
        );
        assert_eq!(
            changes,
            vec![Change::ArgumentBecameRequired {
                type_name: "Query".into(),
                field: "users".into(),
                arg: "limit".into(),
            }]
        );
    }

    #[test]
    fn removed_argument_is_not_breaking() {
        let changes = diff(
            "type Query { users(legacy: String): [User] }",
            "type Query { users: [User] }",
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn removed_enum_value_is_breaking() {
        let changes = diff("enum Status { ACTIVE INACTIVE }", "enum Status { ACTIVE }");
        assert_eq!(
            changes,
            vec![Change::RemovedEnumValue {
                enum_name: "Status".into(),
                value: "INACTIVE".into(),
            }]
        );
    }

    #[test]
    fn new_enum_value_is_not_breaking() {
        let changes = diff("enum Status { ACTIVE }", "enum Status { ACTIVE INACTIVE }");
        assert!(changes.is_empty());
    }

    #[test]
    fn removed_enum_is_breaking() {
        let changes = diff("enum Status { ACTIVE }", "type Empty { x: Int }");
        assert_eq!(
            changes,
            vec![Change::RemovedEnum {
                name: "Status".into()
            }]
        );
    }

    #[test]
    fn identical_schemas_have_no_changes() {
        let schema = "type User { id: ID! name: String } enum Status { ACTIVE INACTIVE }";
        assert!(diff(schema, schema).is_empty());
    }
}
