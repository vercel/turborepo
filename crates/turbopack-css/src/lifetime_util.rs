use lightningcss::{stylesheet::StyleSheet, traits::IntoOwned};

pub fn stylesheet_into_static<'i, 'o>(ss: &StyleSheet) -> StyleSheet<'i, 'o> {
    let sources = ss.sources.clone();

    let rules = ss.rules.clone().into_owned();

    let options = ss.options.clone();

    StyleSheet::new(sources, rules, options)
}
