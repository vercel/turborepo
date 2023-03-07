use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use turbo_tasks::{trace::TraceRawVcs, Value};

use super::request::{
    AdjustFontFallback, NextFontLocalRequest, NextFontLocalRequestArguments, SrcDescriptor,
    SrcRequest,
};

#[turbo_tasks::value(serialization = "auto_for_input")]
#[derive(Clone, Debug, PartialOrd, Ord, Hash)]
pub(crate) struct NextFontLocalOptions {
    pub fonts: Vec<NextFontLocalFontDescriptor>,
    pub display: String,
    pub preload: bool,
    pub fallback: Option<Vec<String>>,
    pub adjust_font_fallback: AdjustFontFallback,
    pub variable: Option<String>,
}

#[turbo_tasks::value_impl]
impl NextFontLocalOptionsVc {
    #[turbo_tasks::function]
    pub fn new(options: Value<NextFontLocalOptions>) -> NextFontLocalOptionsVc {
        Self::cell(options.into_value())
    }
}

#[derive(
    Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TraceRawVcs,
)]
pub(crate) struct NextFontLocalFontDescriptor {
    pub weight: FontWeight,
    pub style: String,
    pub path: String,
    pub ext: String,
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, Hash, TraceRawVcs,
)]
pub(crate) enum FontWeight {
    Variable,
    Fixed(String),
}

// Transforms the request fields to a validated struct.
// Similar to next/font/local's validateData:
// https://github.com/vercel/next.js/blob/28454c6ddbc310419467e5415aee26e48d079b46/packages/font/src/local/utils.ts#L31
pub(crate) fn options_from_request(request: &NextFontLocalRequest) -> Result<NextFontLocalOptions> {
    // Invariant enforced above: either None or Some(the only item in the vec)
    let NextFontLocalRequestArguments {
        display,
        preload,
        fallback,
        src,
        weight,
        style,
        adjust_font_fallback,
        variable,
    } = &request.arguments.0;

    let src_descriptors = match src {
        SrcRequest::Many(d) => d.to_vec(),
        SrcRequest::One(path) => vec![SrcDescriptor {
            path: path.to_owned(),
            weight: weight.to_owned(),
            style: Some(style.to_owned()),
        }],
    };
    let mut fonts = Vec::with_capacity(src_descriptors.len());
    for src_descriptor in src_descriptors {
        let ext = src_descriptor
            .path
            .rsplit('.')
            .next()
            .context("Extension required")?
            .to_owned();

        fonts.push(NextFontLocalFontDescriptor {
            path: src_descriptor.path,
            weight: src_descriptor.weight.map_or_else(
                || FontWeight::Variable,
                |w| {
                    if w == "variable" {
                        FontWeight::Variable
                    } else {
                        FontWeight::Fixed(w)
                    }
                },
            ),
            style: src_descriptor.style.unwrap_or_else(|| style.to_owned()),
            ext,
        });
    }

    Ok(NextFontLocalOptions {
        fonts,
        display: display.to_owned(),
        preload: preload.to_owned(),
        fallback: fallback.to_owned(),
        adjust_font_fallback: adjust_font_fallback.to_owned(),
        variable: variable.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use turbo_tasks_fs::json::parse_json_with_source_context;

    use super::{options_from_request, NextFontLocalOptions};
    use crate::next_font::local::{
        options::{FontWeight, NextFontLocalFontDescriptor},
        request::{AdjustFontFallback, NextFontLocalRequest},
    };

    #[test]
    fn test_uses_defaults() -> Result<()> {
        let request: NextFontLocalRequest = parse_json_with_source_context(
            r#"
            {
                "import": "",
                "path": "index.js",
                "variableName": "myFont",
                "arguments": [{
                    "src": "./Roboto-Regular.ttf"
                }]
            }
        "#,
        )?;

        assert_eq!(
            options_from_request(&request)?,
            NextFontLocalOptions {
                fonts: vec![NextFontLocalFontDescriptor {
                    path: "./Roboto-Regular.ttf".to_owned(),
                    weight: FontWeight::Variable,
                    style: "normal".to_owned(),
                    ext: "ttf".to_owned(),
                }],
                display: "swap".to_owned(),
                preload: true,
                fallback: None,
                adjust_font_fallback: AdjustFontFallback::TimesNewRoman,
                variable: None,
            },
        );

        Ok(())
    }

    #[test]
    fn test_multiple_src() -> Result<()> {
        let request: NextFontLocalRequest = parse_json_with_source_context(
            r#"
            {
                "import": "",
                "path": "index.js",
                "variableName": "myFont",
                "arguments": [{
                    "src": [{
                        "path": "./Roboto-Regular.ttf",
                        "weight": "400",
                        "style": "normal"
                    }, {
                        "path": "./Roboto-Italic.ttf",
                        "weight": "400"
                    }],
                    "weight": "variable",
                    "style": "italic"
                }]
            }
        "#,
        )?;

        assert_eq!(
            options_from_request(&request)?,
            NextFontLocalOptions {
                fonts: vec![
                    NextFontLocalFontDescriptor {
                        path: "./Roboto-Regular.ttf".to_owned(),
                        weight: FontWeight::Fixed("400".to_owned()),
                        style: "normal".to_owned(),
                        ext: "ttf".to_owned(),
                    },
                    NextFontLocalFontDescriptor {
                        path: "./Roboto-Italic.ttf".to_owned(),
                        weight: FontWeight::Fixed("400".to_owned()),
                        style: "italic".to_owned(),
                        ext: "ttf".to_owned(),
                    }
                ],
                display: "swap".to_owned(),
                preload: true,
                fallback: None,
                adjust_font_fallback: AdjustFontFallback::TimesNewRoman,
                variable: None,
            },
        );

        Ok(())
    }

    #[test]
    fn test_true_adjust_fallback_fails() -> Result<()> {
        let request: Result<NextFontLocalRequest> = parse_json_with_source_context(
            r#"
            {
                "import": "",
                "path": "index.js",
                "variableName": "myFont",
                "arguments": [{
                    "src": "./Roboto-Regular.ttf",
                    "adjustFontFallback": true
                }]
            }
        "#,
        );

        match request {
            Ok(r) => panic!("Expected failure, received {:?}", r),
            Err(err) => {
                assert!(err
                    .to_string()
                    .contains("expected Expected string or `false`. Received `true`"),)
            }
        }

        Ok(())
    }

    #[test]
    fn test_specified_options() -> Result<()> {
        let request: NextFontLocalRequest = parse_json_with_source_context(
            r#"
            {
                "import": "",
                "path": "index.js",
                "variableName": "myFont",
                "arguments": [{
                    "src": "./Roboto-Regular.woff",
                    "preload": false,
                    "weight": "500",
                    "style": "italic",
                    "fallback": ["Fallback"],
                    "adjustFontFallback": "Arial",
                    "display": "optional",
                    "variable": "myvar"
                }]
            }
        "#,
        )?;

        assert_eq!(
            options_from_request(&request)?,
            NextFontLocalOptions {
                fonts: vec![NextFontLocalFontDescriptor {
                    path: "./Roboto-Regular.woff".to_owned(),
                    weight: FontWeight::Fixed("500".to_owned()),
                    style: "italic".to_owned(),
                    ext: "woff".to_owned(),
                }],
                display: "optional".to_owned(),
                preload: false,
                fallback: Some(vec!["Fallback".to_owned()]),
                adjust_font_fallback: AdjustFontFallback::Arial,
                variable: Some("myvar".to_owned()),
            },
        );

        Ok(())
    }
}
