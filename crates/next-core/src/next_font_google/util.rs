use std::cmp::Ordering;

use anyhow::{anyhow, bail, Context, Result};
use indexmap::{indexset, IndexSet};
use once_cell::sync::Lazy;
use regex::Regex;

use super::options::{FontData, FontWeights};

static SINGLE_LINE_BLOCK_COMMENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"/\* (.+?) \*/").unwrap());
static GOOGLE_FONT_FILE_URL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"src: url\((.+?)\)").unwrap());

#[derive(Debug, PartialEq)]
pub(crate) struct FontAxes {
    pub(crate) wght: IndexSet<String>,
    pub(crate) ital: IndexSet<FontItal>,
    pub(crate) variable_axes: Option<Vec<(String, String)>>,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) enum FontItal {
    Italic,
    Normal,
}

// Derived from https://github.com/vercel/next.js/blob/9e098da0915a2a4581bebe2270953a1216be1ba4/packages/font/src/google/utils.ts#L232
pub(crate) fn get_font_axes(
    font_data: &FontData,
    font_family: &str,
    weights: &FontWeights,
    styles: &IndexSet<String>,
    selected_variable_axes: &Option<Vec<String>>,
) -> Result<FontAxes> {
    let all_axes = &font_data
        .get(font_family)
        .context("Font family not found")?
        .axes;

    let Some(defineable_axes) = all_axes else {
        bail!("Font {} has no definable `axes`", font_family);
    };

    let ital = {
        let has_italic = styles.contains("italic");
        let has_normal = styles.contains("normal");
        let mut set = IndexSet::new();
        if has_normal {
            set.insert(FontItal::Normal);
        }
        if has_italic {
            set.insert(FontItal::Italic);
        }
        set
    };

    match weights {
        FontWeights::Variable => {
            if let Some(selected_variable_axes) = selected_variable_axes {
                let definable_axes_tags = defineable_axes
                    .iter()
                    .map(|axis| axis.tag.to_owned())
                    .collect::<Vec<String>>();

                for tag in selected_variable_axes {
                    if !definable_axes_tags.contains(tag) {
                        return Err(anyhow!(
                            "Invalid axes value {} for font {}.\nAvailable axes: {}",
                            tag,
                            font_family,
                            definable_axes_tags.join(", ")
                        ));
                    }
                }
            }

            let mut weight_axis = None;
            let mut variable_axes = vec![];
            for axis in defineable_axes {
                if axis.tag == "wght" {
                    weight_axis = Some(format!("{}..{}", axis.min, axis.max));
                } else if let Some(selected_variable_axes) = selected_variable_axes {
                    if selected_variable_axes.contains(&axis.tag) {
                        variable_axes
                            .push((axis.tag.clone(), format!("{}..{}", axis.min, axis.max)));
                    }
                }
            }

            let wght = match weight_axis {
                Some(weight_axis) => {
                    indexset! {weight_axis}
                }
                None => indexset! {},
            };

            Ok(FontAxes {
                wght,
                ital,
                variable_axes: Some(variable_axes),
            })
        }
        FontWeights::Fixed(weights) => Ok(FontAxes {
            wght: weights.clone(),
            ital,
            variable_axes: None,
        }),
    }
}

// Derived from https://github.com/vercel/next.js/blob/9e098da0915a2a4581bebe2270953a1216be1ba4/packages/font/src/google/utils.ts#L128
pub(crate) fn get_stylesheet_url(
    root_url: &str,
    font_family: &str,
    axes: &FontAxes,
    display: &str,
) -> Result<String> {
    // Variants are all combinations of weight and style, each variant will result
    // in a separate font file
    let mut variants: Vec<Vec<(&str, &str)>> = vec![];
    if axes.wght.is_empty() {
        let mut variant = vec![];
        if let Some(variable_axes) = &axes.variable_axes {
            for (key, val) in variable_axes {
                variant.push((key.as_str(), &val[..]));
            }
        }
        variants.push(variant);
    } else {
        for wght in &axes.wght {
            if axes.ital.is_empty() {
                let mut variant = vec![];
                variant.push(("wght", &wght[..]));
                if let Some(variable_axes) = &axes.variable_axes {
                    for (key, val) in variable_axes {
                        variant.push((key, &val[..]));
                    }
                }
                variants.push(variant);
            } else {
                for ital in &axes.ital {
                    let mut variant = vec![];
                    variant.push((
                        "ital",
                        match ital {
                            FontItal::Normal => "0",
                            FontItal::Italic => "1",
                        },
                    ));
                    variant.push(("wght", &wght[..]));
                    if let Some(variable_axes) = &axes.variable_axes {
                        for (key, val) in variable_axes {
                            variant.push((key, &val[..]));
                        }
                    }
                    variants.push(variant);
                }
            }
        }
    }

    for variant in &mut variants {
        // Sort the pairs within the variant by the tag name
        variant.sort_by(|a, b| {
            let is_a_lowercase = a.0.chars().next().unwrap_or_default() as usize > 96;
            let is_b_lowercase = b.0.chars().next().unwrap_or_default() as usize > 96;

            if is_a_lowercase && !is_b_lowercase {
                Ordering::Less
            } else if is_b_lowercase && !is_a_lowercase {
                Ordering::Greater
            } else {
                a.0.cmp(b.0)
            }
        });
    }

    let first_variant = variants
        .first()
        .context("Requires at least one font variant")?;
    // Always use the first variant's keys. There's an implicit invariant from the
    // code above that the keys across each variant are identical, and therefore
    // will be sorted identically across variants.
    //
    // Generates a comma-separated list of axis names, e.g. `ital,opsz,wght`.
    let variant_keys_str = first_variant
        .iter()
        .map(|pair| pair.0)
        .collect::<Vec<&str>>()
        .join(",");

    let mut variant_values = variants
        .iter()
        .map(|variant| {
            variant
                .iter()
                .map(|pair| pair.1)
                .collect::<Vec<&str>>()
                .join(",")
        })
        .collect::<Vec<String>>();
    variant_values.sort();
    // An encoding of the series of sorted variant values, with variants delimited
    // by `;` and the values within a variant delimited by `,` e.g.
    // `"0,10..100,500;1,10.100;500"`
    let variant_values_str = variant_values.join(";");

    Ok(format!(
        "{}?family={}:{}@{}&display={}",
        root_url,
        font_family.replace(' ', "+"),
        variant_keys_str,
        variant_values_str,
        display
    ))
}

#[derive(Debug, PartialEq)]
pub(crate) struct ExtractedFonts {
    pub(crate) all_urls: IndexSet<String>,
    pub(crate) preload_urls: IndexSet<String>,
}

// Derived from https://github.com/vercel/next.js/blob/b0aa73b4cf23cb77bd492cfed7624d5cfbbd4990/packages/font/src/google/loader.ts#L114
pub(crate) fn extract_font_urls(
    stylesheet: &str,
    subsets: Option<&Vec<String>>,
    should_preload_subsets: bool,
) -> Result<ExtractedFonts> {
    let mut all_urls = IndexSet::new();
    let mut preload_urls = IndexSet::new();

    let mut current_subset = None;
    for line in stylesheet.split('\n') {
        let new_subset = SINGLE_LINE_BLOCK_COMMENT_RE
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|m| m.as_str());

        match new_subset {
            Some(subset) => {
                current_subset = Some(subset);
            }
            None => {
                let font_url = GOOGLE_FONT_FILE_URL_RE
                    .captures(line)
                    .and_then(|captures| captures.get(1))
                    .map(|m| m.as_str());

                if let Some(url) = font_url {
                    if !all_urls.contains(url) {
                        all_urls.insert(url.to_owned());

                        let should_preload = match subsets {
                            Some(subsets) => {
                                should_preload_subsets
                                    && subsets.contains(
                                        &current_subset
                                            .context(
                                                "Invariant: subset should be set by preceeding \
                                                 comment at this point",
                                            )?
                                            .to_owned(),
                                    )
                            }
                            None => false,
                        };
                        if should_preload {
                            preload_urls.insert(url.to_owned());
                        }
                    }
                }
            }
        }
    }

    Ok(ExtractedFonts {
        all_urls,
        preload_urls,
    })
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use indexmap::indexset;

    use super::{extract_font_urls, get_font_axes};
    use crate::next_font_google::{
        options::{FontData, FontWeights},
        util::{get_stylesheet_url, ExtractedFonts, FontAxes, FontItal},
        GOOGLE_FONTS_STYLESHEET_URL,
    };

    #[test]
    fn test_errors_on_unknown_font() -> Result<()> {
        let data: FontData = serde_json::from_str(
            r#"
            {
                "ABeeZee": {
                    "weights": ["variable"],
                    "styles": ["normal", "italic"]
                }
            }
  "#,
        )?;

        match get_font_axes(
            &data,
            "foobar",
            &FontWeights::Variable,
            &indexset! {},
            &None,
        ) {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.to_string(), "Font family not found")
            }
        }
        Ok(())
    }

    #[test]
    fn test_errors_on_missing_axes() -> Result<()> {
        let data: FontData = serde_json::from_str(
            r#"
            {
                "ABeeZee": {
                    "weights": ["variable"],
                    "styles": ["normal", "italic"]
                }
            }
  "#,
        )?;

        match get_font_axes(
            &data,
            "ABeeZee",
            &FontWeights::Variable,
            &indexset! {},
            &None,
        ) {
            Ok(_) => panic!(),
            Err(err) => {
                assert_eq!(err.to_string(), "Font ABeeZee has no definable `axes`")
            }
        }
        Ok(())
    }

    #[test]
    fn test_selecting_axes() -> Result<()> {
        let data: FontData = serde_json::from_str(
            r#"
            {
                "Inter": {
                    "weights": [
                        "400",
                        "variable"
                    ],
                    "styles": ["normal", "italic"],
                    "axes": [
                        {
                            "tag": "slnt",
                            "min": -10,
                            "max": 0,
                            "defaultValue": 0
                        },
                        {
                            "tag": "wght",
                            "min": 100,
                            "max": 900,
                            "defaultValue": 400
                        }
                    ]
                }
            }
  "#,
        )?;

        assert_eq!(
            get_font_axes(
                &data,
                "Inter",
                &FontWeights::Variable,
                &indexset! {},
                &Some(vec!["slnt".to_owned()]),
            )?,
            FontAxes {
                wght: indexset! {"100..900".to_owned()},
                ital: indexset! {},
                variable_axes: Some(vec![("slnt".to_owned(), "-10..0".to_owned())])
            }
        );
        Ok(())
    }

    #[test]
    fn test_no_wght_axis() -> Result<()> {
        let data: FontData = serde_json::from_str(
            r#"
            {
                "Inter": {
                    "weights": [
                        "400",
                        "variable"
                    ],
                    "styles": ["normal", "italic"],
                    "axes": [
                        {
                            "tag": "slnt",
                            "min": -10,
                            "max": 0,
                            "defaultValue": 0
                        }
                    ]
                }
            }
  "#,
        )?;

        assert_eq!(
            get_font_axes(
                &data,
                "Inter",
                &FontWeights::Variable,
                &indexset! {},
                &Some(vec!["slnt".to_owned()]),
            )?,
            FontAxes {
                wght: indexset! {},
                ital: indexset! {},
                variable_axes: Some(vec![("slnt".to_owned(), "-10..0".to_owned())])
            }
        );
        Ok(())
    }

    #[test]
    fn test_stylesheet_url_no_axes() -> Result<()> {
        assert_eq!(
            get_stylesheet_url(
                GOOGLE_FONTS_STYLESHEET_URL,
                "Roboto Mono",
                &FontAxes {
                    wght: indexset! {"500".to_owned()},
                    ital: indexset! {FontItal::Normal},
                    variable_axes: None
                },
                "optional"
            )?,
            "https://fonts.googleapis.com/css2?family=Roboto+Mono:ital,wght@0,500&display=optional"
        );

        Ok(())
    }

    #[test]
    fn test_stylesheet_url_sorts_axes() -> Result<()> {
        assert_eq!(
            get_stylesheet_url(
                GOOGLE_FONTS_STYLESHEET_URL,
                "Roboto Serif",
                &FontAxes {
                    wght: indexset! {"500".to_owned()},
                    ital: indexset! {FontItal::Normal},
                    variable_axes: Some(vec![
                        ("GRAD".to_owned(), "-50..100".to_owned()),
                        ("opsz".to_owned(), "8..144".to_owned()),
                        ("wdth".to_owned(), "50..150".to_owned()),
                    ])
                },
                "optional"
            )?,
            "https://fonts.googleapis.com/css2?family=Roboto+Serif:ital,opsz,wdth,wght,GRAD@0,8..144,50..150,500,-50..100&display=optional"
        );

        Ok(())
    }

    #[test]
    fn test_stylesheet_url_encodes_all_weight_ital_combinations() -> Result<()> {
        assert_eq!(
            get_stylesheet_url(
                GOOGLE_FONTS_STYLESHEET_URL,
                "Roboto Serif",
                &FontAxes {
                    wght: indexset! {"500".to_owned(), "300".to_owned()},
                    ital: indexset! {FontItal::Normal, FontItal::Italic},
                    variable_axes: Some(vec![
                        ("GRAD".to_owned(), "-50..100".to_owned()),
                        ("opsz".to_owned(), "8..144".to_owned()),
                        ("wdth".to_owned(), "50..150".to_owned()),
                    ])
                },
                "optional"
            )?,
            // Note ;-delimited sections for normal@300, normal@500, italic@300, italic@500
            "https://fonts.googleapis.com/css2?family=Roboto+Serif:ital,opsz,wdth,wght,GRAD@0,8..144,50..150,300,-50..100;0,8..144,50..150,500,-50..100;1,8..144,50..150,300,-50..100;1,8..144,50..150,500,-50..100&display=optional"
        );

        Ok(())
    }

    #[test]
    fn test_variable_font_without_wgth_axis() -> Result<()> {
        assert_eq!(
            get_stylesheet_url(
                GOOGLE_FONTS_STYLESHEET_URL,
                "Nabla",
                &FontAxes {
                    wght: indexset! {},
                    ital: indexset! {},
                    variable_axes: Some(vec![
                        ("EDPT".to_owned(), "0..200".to_owned()),
                        ("EHLT".to_owned(), "0..24".to_owned()),
                    ])
                },
                "optional"
            )?,
            "https://fonts.googleapis.com/css2?family=Nabla:EDPT,EHLT@0..200,0..24&display=optional"
        );

        Ok(())
    }

    #[test]
    fn test_extract_font_urls_preloads_subsets() -> Result<()> {
        assert_eq!(
            // From https://fonts.googleapis.com/css2?family=Roboto+Serif:ital,opsz,wdth,wght,GRAD@0,8..144,50..150,300,-50..100%253B0,8..144,50..150,500,-50..100%253B1,8..144,50..150,300,-50..100%253B1,8..144,50..150,500,-50..100&display=optional
            extract_font_urls(
                r#"/* cyrillic-ext */
@font-face {
  font-family: 'Roboto Serif';
  font-style: italic;
  font-weight: 300;
  font-stretch: 50% 150%;
  font-display: optional;
  src: url(https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSPzb5CTn22byl0.woff2) format('woff2');
  unicode-range: U+0460-052F, U+1C80-1C88, U+20B4, U+2DE0-2DFF, U+A640-A69F, U+FE2E-FE2F;
}
/* vietnamese */
@font-face {
  font-family: 'Roboto Serif';
  font-style: italic;
  font-weight: 300;
  font-stretch: 50% 150%;
  font-display: optional;
  src: url(https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSPxb5CTn22byl0.woff2) format('woff2');
  unicode-range: U+0102-0103, U+0110-0111, U+0128-0129, U+0168-0169, U+01A0-01A1, U+01AF-01B0, U+1EA0-1EF9, U+20AB;
}
/* latin-ext */
@font-face {
  font-family: 'Roboto Serif';
  font-style: italic;
  font-weight: 300;
  font-stretch: 50% 150%;
  font-display: optional;
  src: url(https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSPwb5CTn22byl0.woff2) format('woff2');
  unicode-range: U+0100-024F, U+0259, U+1E00-1EFF, U+2020, U+20A0-20AB, U+20AD-20CF, U+2113, U+2C60-2C7F, U+A720-A7FF;
}
/* latin */
@font-face {
  font-family: 'Roboto Serif';
  font-style: italic;
  font-weight: 300;
  font-stretch: 50% 150%;
  font-display: optional;
  src: url(https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSP-b5CTn22b.woff2) format('woff2');
  unicode-range: U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+2000-206F, U+2074, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD;
}
"#,
                Some(vec!["latin".to_owned()]).as_ref(),
                true
            )?,
            ExtractedFonts {
                all_urls: indexset! {
                    // All very similar, but unique urls
                    "https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSPzb5CTn22byl0.woff2".to_owned(),
                    "https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSPxb5CTn22byl0.woff2".to_owned(),
                    "https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSPwb5CTn22byl0.woff2".to_owned(),
                    "https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSP-b5CTn22b.woff2".to_owned()
                },
                preload_urls: indexset! {
                    "https://fonts.gstatic.com/s/robotoserif/v8/R70DjywflP6FLr3gZx7K8UyEVSP-b5CTn22b.woff2".to_owned()
                }
            }
        );

        Ok(())
    }
}
