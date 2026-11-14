#[test]
fn converts_with_the_shipped_rate() {
    let eur = rates::rate("EUR").unwrap();
    assert_eq!(rates::to_cents(1, eur), eur);
}
