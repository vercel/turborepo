mod common;

#[test]
fn test_boundaries() -> Result<(), anyhow::Error> {
    check_json!(
        "boundaries",
        "npm@10.5.0",
        "query",
        "get boundaries lints" => "query { boundaries { items { message import } } }",
    );

    Ok(())
}
