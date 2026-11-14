#[test]
fn reads_fixture() {
    let _ = std::fs::read_to_string("tests/data.txt");
}
