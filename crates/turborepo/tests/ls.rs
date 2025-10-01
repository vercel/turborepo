mod common;

#[test]
fn test_query() -> Result<(), anyhow::Error> {
    check_json_output!(
        "basic_monorepo",
        "npm@10.5.0",
        "ls",
        "get packages" => ["--output=json"],
        "get info for package `my-app`" => ["my-app", "--output=json"],
    );

    Ok(())
}
