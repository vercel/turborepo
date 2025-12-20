//! Repository detection and package discovery for Turborepo.
//! Handles monorepo structure, package graph construction, and dependency
//! analysis.
//!
//! Primarily in a separate crate from the rest of the logic so the
//! `@turbo/repository` NPM package can avoid depending on the entire Turborepo
//! binary.

#![feature(assert_matches)]
#![feature(error_generic_member_access)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]
#![allow(clippy::result_large_err)]

pub mod change_mapper;
pub mod discovery;
pub mod inference;
pub mod package_graph;
pub mod package_json;
pub mod package_manager;
pub mod workspaces;
