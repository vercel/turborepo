use turborepo_vt100 as vt100;

mod helpers;

#[test]
fn modes() {
    helpers::fixture("modes");
}

#[test]
fn alternate_buffer() {
    helpers::fixture("alternate_buffer");
}
