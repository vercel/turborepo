#![feature(assert_matches)]
#![feature(error_generic_member_access)]
#![allow(clippy::result_large_err)]

pub mod change_mapper;
pub mod discovery;
pub mod inference;
pub mod package_graph;
pub mod package_json;
pub mod package_manager;
pub mod workspaces;
