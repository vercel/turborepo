use anyhow::{anyhow, Context, Result};
use indexmap::{indexset, IndexSet};

use super::options::{FontData, FontWeights};

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
        return Err(anyhow!("Font {} has no definable `axes`", font_family));
    };

    let has_italic = styles.contains("italic");
    let has_normal = styles.contains("normal");
    let ital = {
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

            let Some(weight_axis) = weight_axis else {
                return Err(anyhow!("Expected wght axis to appear in font data for {}", font_family));
            };

            Ok(FontAxes {
                wght: indexset! {weight_axis},
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use indexmap::indexset;

    use super::get_font_axes;
    use crate::next_font_google::{
        options::{FontData, FontWeights},
        util::{FontAxes, FontItal},
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
}
