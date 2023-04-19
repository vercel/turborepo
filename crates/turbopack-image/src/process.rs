use std::io::Cursor;

use anyhow::{bail, Context, Result};
use base64::{display::Base64Display, engine::general_purpose::STANDARD};
use image::{
    codecs::{
        jpeg::JpegEncoder,
        png::{CompressionType, PngEncoder},
        webp::{WebPEncoder, WebPQuality},
    },
    imageops::FilterType,
    GenericImageView, ImageEncoder, ImageFormat,
};
use turbo_tasks::primitives::StringVc;
use turbo_tasks_fs::{FileContent, FileContentVc, FileSystemPathVc};
use turbopack_core::{
    error::PrettyPrintError,
    ident::AssetIdentVc,
    issue::{Issue, IssueVc},
};

#[turbo_tasks::value]
#[serde(rename_all = "camelCase")]
pub struct ImageMetaData {
    width: u32,
    height: u32,
    #[serde(rename = "blurDataURL")]
    blur_data_url: Option<String>,
    blur_width: u32,
    blur_height: u32,
}

const BLUR_IMG_SIZE: u32 = 8;
const BLUR_QUALITY: u8 = 70;

#[turbo_tasks::function]
pub async fn get_meta_data_and_blur_placeholder(
    ident: AssetIdentVc,
    content: FileContentVc,
) -> Result<ImageMetaDataVc> {
    let FileContent::Content(content) = &*content.await? else {
      bail!("Input image not found");
    };
    let bytes = content.content().to_bytes()?;
    fn load_image(bytes: &[u8]) -> Result<(image::DynamicImage, Option<ImageFormat>)> {
        let reader = image::io::Reader::new(Cursor::new(&bytes));
        let reader = reader
            .with_guessed_format()
            .context("unable to determine image format from file content")?;
        let format = reader.format();
        let image = reader.decode().context("unable to decode image data")?;
        Ok((image, format))
    }
    let (image, format) = match load_image(&bytes) {
        Ok(r) => r,
        Err(err) => {
            ImageProcessingIssue {
                path: ident.path(),
                message: StringVc::cell(format!("{}", PrettyPrintError(&err))),
            }
            .cell()
            .as_issue()
            .emit();
            return Ok(ImageMetaData {
                width: 0,
                height: 0,
                blur_data_url: None,
                blur_width: 0,
                blur_height: 0,
            }
            .cell());
        }
    };
    let (width, height) = image.dimensions();
    let (blur_data_url, blur_width, blur_height) = if matches!(
        format,
        // list should match next/client/image.tsx
        Some(ImageFormat::Png)
            | Some(ImageFormat::Jpeg)
            | Some(ImageFormat::WebP)
            | Some(ImageFormat::Avif)
    ) {
        fn compute_blur_data(
            image: image::DynamicImage,
            format: ImageFormat,
        ) -> Result<(String, u32, u32)> {
            let small_image = image.resize(BLUR_IMG_SIZE, BLUR_IMG_SIZE, FilterType::Triangle);
            let mut buf = Vec::new();
            let blur_width = small_image.width();
            let blur_height = small_image.height();
            let url = match format {
                ImageFormat::Png => {
                    PngEncoder::new_with_quality(
                        &mut buf,
                        CompressionType::Best,
                        image::codecs::png::FilterType::NoFilter,
                    )
                    .write_image(
                        small_image.as_bytes(),
                        blur_width,
                        blur_height,
                        small_image.color(),
                    )?;
                    format!(
                        "data:image/png;base64,{}",
                        Base64Display::new(&buf, &STANDARD)
                    )
                }
                ImageFormat::Jpeg => {
                    JpegEncoder::new_with_quality(&mut buf, BLUR_QUALITY).write_image(
                        small_image.as_bytes(),
                        blur_width,
                        blur_height,
                        small_image.color(),
                    )?;
                    format!(
                        "data:image/jpeg;base64,{}",
                        Base64Display::new(&buf, &STANDARD)
                    )
                }
                ImageFormat::WebP => {
                    WebPEncoder::new_with_quality(&mut buf, WebPQuality::lossy(BLUR_QUALITY))
                        .write_image(
                            small_image.as_bytes(),
                            blur_width,
                            blur_height,
                            small_image.color(),
                        )?;
                    format!(
                        "data:image/webp;base64,{}",
                        Base64Display::new(&buf, &STANDARD)
                    )
                }
                #[cfg(feature = "avif")]
                ImageFormat::Avif => {
                    use image::codecs::avif::AvifEncoder;
                    AvifEncoder::new_with_speed_quality(&mut buf, 6, BLUR_QUALITY).write_image(
                        small_image.as_bytes(),
                        blur_width,
                        blur_height,
                        small_image.color(),
                    )?;
                    format!(
                        "data:image/avif;base64,{}",
                        Base64Display::new(&buf, &STANDARD)
                    )
                }
                _ => unreachable!(),
            };

            Ok((url, blur_width, blur_height))
        }

        match compute_blur_data(image, format.unwrap())
            .context("unable to compute blur placeholder")
        {
            Ok((url, blur_width, blur_height)) => (Some(url), blur_width, blur_height),
            Err(err) => {
                ImageProcessingIssue {
                    path: ident.path(),
                    message: StringVc::cell(format!("{}", PrettyPrintError(&err))),
                }
                .cell()
                .as_issue()
                .emit();
                (None, 0, 0)
            }
        }
    } else {
        (None, 0, 0)
    };

    Ok(ImageMetaData {
        width,
        height,
        blur_data_url,
        blur_width,
        blur_height,
    }
    .cell())
}

#[turbo_tasks::value]
struct ImageProcessingIssue {
    path: FileSystemPathVc,
    message: StringVc,
}

#[turbo_tasks::value_impl]
impl Issue for ImageProcessingIssue {
    #[turbo_tasks::function]
    fn context(&self) -> FileSystemPathVc {
        self.path
    }
    #[turbo_tasks::function]
    fn category(&self) -> StringVc {
        StringVc::cell("image".to_string())
    }
    #[turbo_tasks::function]
    fn title(&self) -> StringVc {
        StringVc::cell("Processing image failed".to_string())
    }
    #[turbo_tasks::function]
    fn description(&self) -> StringVc {
        self.message
    }
}
