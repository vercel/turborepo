use std::path::{Path, PathBuf};

use next_transform_dynamic::{next_dynamic, NextDynamicMode};
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

#[fixture("tests/webpack/errors/**/input.js")]
fn next_dynamic_webpack_errors(input: PathBuf) {
    next_dynamic_errors(&input, NextDynamicMode::Webpack);
}

#[fixture("tests/turbo/errors/**/input.js")]
fn next_dynamic_turbo_errors(input: PathBuf) {
    next_dynamic_errors(&input, NextDynamicMode::Turbo);
}

fn next_dynamic_errors(input: &Path, mode: NextDynamicMode) {
    let output = input.parent().unwrap().join("output.js");
    test_fixture(
        syntax(),
        &|_tr| {
            next_dynamic(
                true,
                false,
                false,
                mode,
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
