# gqldiff

Diffs two GraphQL SDL schema files and reports breaking changes — the
same job `buf breaking` does for protobuf (see this repo's own
`protodiff`), but for a GraphQL schema instead of a `.proto` file.
GraphQL's own ecosystem has `graphql-inspector` (Node) for this; there's
no standalone Rust equivalent that doesn't require pulling in a full
`graphql-parser`/`async-graphql` dependency tree just to diff two files.

## Usage

```bash
gqldiff old-schema.graphql new-schema.graphql
```

```
$ gqldiff schema_v1.graphql schema_v2.graphql
7 breaking change(s):
  Comment.author: type changed from User! to ID!
  Mutation.createUser: argument 'notify' was optional, is now required
  type 'Node' removed
  Query.users: new required argument 'activeOnly' — existing callers don't send it
  User.name: type changed from String to String!
  User.age removed
  Status: enum value 'PENDING' removed
```

Exit code `1` if any breaking change is found — drop it in CI comparing
the schema on `main` against a PR branch's schema (any GraphQL server
framework can dump its schema via SDL introspection as a build step,
the same way `apidiff` expects an OpenAPI JSON dump). A clean run
(including the same file against itself) prints `no breaking changes`
and exits `0`.

## What counts as breaking (and what doesn't)

| Change | Breaking? |
|---|---|
| Type, interface, or input removed | yes |
| New type | no — additive |
| Field removed | yes |
| New optional field | no — additive |
| Field's type changed (`String` -> `Int`) | yes |
| Field's type tightened (`String` -> `String!`) | yes |
| Field's type loosened (`String!` -> `String`) | yes — still a type-signature change existing codegen clients can choke on, see below |
| New **required** argument (non-null, no default) added to an existing field | yes |
| New **optional** argument, or a new required argument **with a default** | no — additive |
| Existing argument becomes required | yes |
| Argument removed | no — a caller that stops sending it is fine |
| Enum value removed | yes |
| New enum value | no — additive |

The one row that looks debatable: `String!` loosening to `String` is
reported as a change rather than silently allowed, even though a raw
GraphQL client sending less-strict queries wouldn't notice. This tool's
real audience is codegen'd clients (`graphql-codegen`, Apollo Client's
generated types, etc.) where a field going from guaranteed-non-null to
possibly-null is a real generated-type change, not a no-op — so it's
flagged the same as any other type-signature change rather than
special-cased as safe. It's a judgment call, and it's stated here
plainly rather than left as a surprise.

## Status: built and verified against a realistic before/after schema with a genuine mix of 7 breaking and 7 safe changes

- **28 unit tests** (`cargo test --lib`): SDL parsing (field/argument
  extraction, list and nested-nullability type signatures like
  `[String!]!`, default-value presence tracked correctly without
  needing to parse the default's actual value, block (`"""..."""`) and
  single-line (`"..."`) descriptions stripped without corrupting the
  field that follows, `#` line comments stripped, `@deprecated(...)`
  directive usages stripped, `interface`/`input` parsed as field
  containers the same as `type`, an `implements Node` clause not
  breaking the type-header match); and every row of the breaking/safe
  table above as its own test, including the two easy-to-get-backwards
  pairs (a new required argument *without* a default is breaking, the
  same shape *with* a default isn't; an existing argument going
  optional -> required is breaking, the reverse isn't flagged at all).
  One test is a direct regression-shaped check in the spirit of
  `protodiff`'s own documented single-line-parsing bug: multiple
  `type`/`enum` declarations packed onto one line all parse correctly,
  because the type/enum header regexes match on a `\b` word boundary
  rather than a line-start anchor for exactly that reason.
- **CLI run against a realistic before/after pair** (`schema_v1.graphql`
  -> `schema_v2.graphql`, an evolving `User`/`Comment`/`Query`/
  `Mutation` schema): 7 real breaking changes introduced in the same
  diff as 7 real safe/additive changes (a new optional field, a
  brand-new type, a new enum value, two new optional arguments, and a
  new required argument that carries a default). All 7 breaking changes
  were reported by name, all 7 safe changes were confirmed absent from
  the output by grepping for each one, and the same file diffed against
  itself printed `no breaking changes` with exit `0`.

**Not done / deliberately deferred**: this is a hand-rolled
regex/brace-depth SDL scanner, not a full GraphQL grammar. `type`,
`input`, and `interface` are pooled into one internal model since they
share identical `name: Type` field syntax — a real gap is that this
means an `interface` and a `type` removal are reported with the same
generic `type 'X' removed` wording rather than distinguishing kind, and
an input type's field being added/removed is scored with the same
"add is safe / remove is breaking" direction as an output type, when
in a stricter GraphQL server an *unexpected extra* input field can
itself be a validation error — that asymmetry isn't modeled. `union`
and `scalar` declarations, `schema { query: ... }` blocks, custom
directive *definitions* (`directive @foo on FIELD`), and `extend type`
are not parsed at all (silently ignored, not an error, the same way
`protodiff` ignores unresolved cross-file imports). No `$ref`-style
schema stitching or multi-file `#import` support. Field/argument
descriptions and default *values* are discarded during parsing —
whether a default exists is tracked, its actual value never is, since
none of this tool's breaking-change rules need it.
