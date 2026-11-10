//! Code generator recognizers (Prisma, Drizzle, graphql-codegen, buf/protoc,
//! OpenAPI generators, sqlc, `//go:generate` tools, Python generators) and
//! generated-file headers.
//!
//! See [`crate::registry`] for the [`Recognizer`]
//! contract: a generator's schema/config files become inputs or configs, and
//! what it writes becomes [`crate::spec::Output`]s, which the store links back
//! to the task (generated-file linking), so `schema.prisma` →
//! `node_modules/.prisma/client` → importers → tests.
//!
//! | Module | Programs | Config read statically |
//! |---|---|---|
//! | spec `prisma` | `prisma generate\|migrate\|db\|format\|validate` | `--schema`, `prisma.config.*`, `package.json#prisma`, schema blocks ([`crate::scanners::prisma`]) |
//! | spec `drizzle-kit` | `drizzle-kit generate\|push\|migrate\|pull\|check\|...` | `drizzle.config.*` `schema`, `out` |
//! | [`graphql`] | `graphql-codegen`, `gql-gen`, `graphql-code-generator` | `codegen.{yml,yaml,json,ts,js}`, `.codegenrc*`, `package.json#codegen`, graphql-config |
//! | [`protobuf`] | `buf generate\|build\|lint\|...`, `protoc`, `grpc_tools.protoc`, `grpc_tools_node_protoc`, `protoc-gen-*` | `buf.gen.yaml`, `buf.yaml`, `buf.work.yaml`, proto imports |
//! | specs `openapi-typescript`, `orval` | `openapi-typescript`, `orval` | `redocly.yaml`, `orval.config.*`, spec `$ref`s ([`crate::scanners::refs`]) |
//! | [`openapi`] | `openapi-generator(-cli)`, `openapi-ts`, `swagger-typescript-api`, `openapi` (openapi-typescript-codegen) | `openapitools.json`, `openapi-ts.config.*`, spec `$ref`s |
//! | spec `sqlc` | `sqlc generate\|vet\|compile\|diff\|verify` | `sqlc.{yaml,yml,json}` |
//! | [`golang`] | `go generate`, `go run <tool>`, `go tool <tool>`, `stringer`, `enumer`, `mockgen`, `oapi-codegen`, `easyjson`, `ent`, `wire`, `gqlgen` | `//go:generate` directives, oapi-codegen config, `gqlgen.yml` |
//! | [`python`] | `datamodel-codegen`, `strawberry export-schema` | `pyproject.toml [tool.datamodel-codegen]` |
//! | [`headers`] | (not a recognizer) | `Code generated … DO NOT EDIT.`, `@generated`, `This file was automatically generated` |
//!
//! Specs are TOML files in `crates/turborepo-trace-tasks/specs/`, interpreted
//! by [`crate::specs`].
//!
//! Whatever is not statically known (an env lookup in a config, a remote schema
//! URL, a generator without a known default output) becomes an uncertainty plus
//! a membership on the config's directory ([`config::unknown`]); the registry
//! adds the package directory too.

pub mod config;
pub mod golang;
pub mod graphql;
pub mod headers;
pub mod openapi;
pub mod protobuf;
pub mod python;

use crate::{
    registry::{Recognition, Recognizer},
    spec::TaskSpec,
};

/// Code generator recognizers (registered first).
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    let mut v: Vec<&'static dyn Recognizer> = Vec::new();
    v.extend(crate::specs::recognizers());
    v.extend(graphql::recognizers());
    v.extend(protobuf::recognizers());
    v.extend(openapi::recognizers());
    v.extend(golang::recognizers());
    v.extend(python::recognizers());
    v
}

/// `Full` unless uncertainties were added since `before` (then `Partial`).
pub(crate) fn finish(out: &TaskSpec, before: usize) -> Recognition {
    if out.uncertainties.len() > before {
        Recognition::Partial
    } else {
        Recognition::Full
    }
}
