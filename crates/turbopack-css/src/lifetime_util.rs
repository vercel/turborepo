use lightningcss::{rules::import::ImportRule, stylesheet::StyleSheet};

pub fn stylesheet_into_static<'i, 'o>(_ss: &StyleSheet) -> StyleSheet<'i, 'o> {
    todo!()
}

pub fn import_rule_to_static<'i>(_i: &ImportRule) -> ImportRule<'i> {
    todo!()
}
