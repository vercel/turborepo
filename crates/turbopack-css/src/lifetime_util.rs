use lightningcss::{
    stylesheet::{ParserOptions, StyleSheet},
    traits::IntoOwned,
};

pub fn stylesheet_into_static<'i, 'o>(
    ss: &StyleSheet,
    options: ParserOptions<'o, 'i>,
) -> StyleSheet<'i, 'o> {
    let sources = ss.sources.clone();
    dbg!("stylesheet_into_static::after sources.clone", &ss.sources);
    let rules = ss.rules.clone().into_owned();
    dbg!("stylesheet_into_static::after rules.clone", &ss.sources);

    StyleSheet::new(sources, rules, options)
}
