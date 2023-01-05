use std::path::PathBuf;

use next_transform_dynamic::next_dynamic;
use swc_core::{
    common::FileName,
    ecma::{
        parser::{EsConfig, Syntax},
        transforms::testing::{test_fixture, FixtureTestConfig},
    },
};
use testing::fixture;

fn syntax() -> Syntax {
    Syntax::Es(EsConfig {
        jsx: true,
        ..Default::default()
    })
}

#[fixture("tests/errors/**/input.js")]
fn next_dynamic_errors(input: PathBuf) {
    let output = input.parent().unwrap().join("output.js");
    test_fixture(
        syntax(),
        &|_tr| {
            next_dynamic(
                true,
                false,
                false,
                FileName::Real(PathBuf::from("/some-project/src/some-file.js")),
                Some("/some-project/src".into()),
            )
        },
        &input,
        &output,
        FixtureTestConfig {
            allow_error: true,
            ..Default::default()
        },
    );
}
