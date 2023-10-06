use lightningcss::{
    rules::{import::ImportRule, CssRule, CssRuleList},
    stylesheet::StyleSheet,
};

pub fn stylesheet_into_static<'i, 'o>(ss: &StyleSheet) -> StyleSheet<'i, 'o> {
    let sources = ss.sources.clone();

    let rules = CssRuleList(
        ss.rules
            .0
            .into_iter()
            .map(|rule| css_rule_to_static(&rule))
            .collect(),
    );

    let options = ss.options.clone();

    StyleSheet::new(sources, rules, options)
}

fn css_rule_to_static<'i>(r: &CssRule) -> CssRule<'i> {
    match r {
        CssRule::Media(r) => {}
        CssRule::Import(r) => CssRule::Import(r.clone().into_owned()),
        CssRule::Style(r) => {}
        CssRule::Keyframes(r) => {}
        CssRule::FontFace(r) => {}
        CssRule::FontPaletteValues(r) => {}
        CssRule::Page(r) => {}
        CssRule::Supports(r) => {}
        CssRule::CounterStyle(r) => {}
        CssRule::Namespace(r) => {}
        CssRule::MozDocument(r) => {}
        CssRule::Nesting(r) => {}
        CssRule::Viewport(r) => {}
        CssRule::CustomMedia(r) => {}
        CssRule::LayerStatement(r) => {}
        CssRule::LayerBlock(r) => {}
        CssRule::Property(r) => {}
        CssRule::Container(r) => {}
        CssRule::StartingStyle(r) => {}
        CssRule::Ignored => todo!(),
        CssRule::Unknown(r) => {}
        CssRule::Custom(r) => {}
    }
}
