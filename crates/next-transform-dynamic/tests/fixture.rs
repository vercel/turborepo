use std::path::PathBuf;

use next_transform_dynamic::next_dynamic;
use swc_core::{
    common::FileName,
    ecma::{
        parser::{EsConfig, Syntax},
        transforms::testing::{test, test_fixture},
    },
};
use testing::fixture;

fn syntax() -> Syntax {
    Syntax::Es(EsConfig {
        jsx: true,
        ..Default::default()
    })
}

#[fixture("tests/fixture/**/input.js")]
fn next_dynamic_fixture(input: PathBuf) {
    let output_dev = input.parent().unwrap().join("output-dev.js");
    let output_prod = input.parent().unwrap().join("output-prod.js");
    let output_server = input.parent().unwrap().join("output-server.js");
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
        &output_dev,
        Default::default(),
    );
    test_fixture(
        syntax(),
        &|_tr| {
            next_dynamic(
                false,
                false,
                false,
                FileName::Real(PathBuf::from("/some-project/src/some-file.js")),
                Some("/some-project/src".into()),
            )
        },
        &input,
        &output_prod,
        Default::default(),
    );
    test_fixture(
        syntax(),
        &|_tr| {
            next_dynamic(
                false,
                true,
                false,
                FileName::Real(PathBuf::from("/some-project/src/some-file.js")),
                Some("/some-project/src".into()),
            )
        },
        &input,
        &output_server,
        Default::default(),
    );
}
