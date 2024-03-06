mod helpers;

#[test]
fn title() {
    helpers::fixture("title");
}

#[test]
fn icon_name() {
    helpers::fixture("icon_name");
}

#[test]
fn title_icon_name() {
    helpers::fixture("title_icon_name");
}

#[test]
fn unknown_osc() {
    helpers::fixture("unknown_osc");
}
