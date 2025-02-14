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

#[test]
fn test_boundaries_tags() -> Result<(), anyhow::Error> {
    check_json!(
        "boundaries_tags",
        "npm@10.5.0",
        "query",
        "get boundaries lints" => "query { boundaries { items { message import } } }",
    );

    Ok(())
}

#[test]
fn test_boundaries_on_basic_monorepo() -> Result<(), anyhow::Error> {
    check_json!(
        "basic_monorepo",
        "npm@10.5.0",
        "query",
        "get boundaries lints" => "query { boundaries { items { message import } } }",
    );

    Ok(())
}
