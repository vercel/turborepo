use std::io::Cursor;

use anyhow::{bail, Result};
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
use turbo_tasks_fs::{FileContent, FileContentVc};

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
pub async fn get_meta_data_and_blur_placeholder(content: FileContentVc) -> Result<ImageMetaDataVc> {
    let FileContent::Content(content) = &*content.await? else {
      bail!("Input image not found");
    };
    let bytes = content.content().to_bytes()?;
    let reader = image::io::Reader::new(Cursor::new(&bytes));
    let reader = reader.with_guessed_format()?;
    let format = reader.format();
    let image = reader.decode()?;
    let (width, height) = image.dimensions();
    let (blur_data_url, blur_width, blur_height) = if matches!(
        format,
        // list should match next/client/image.tsx
        Some(ImageFormat::Png)
            | Some(ImageFormat::Jpeg)
            | Some(ImageFormat::WebP)
            | Some(ImageFormat::Avif)
    ) {
        let small_image = image.resize(BLUR_IMG_SIZE, BLUR_IMG_SIZE, FilterType::Triangle);
        let mut buf = Vec::new();
        let blur_width = small_image.width();
        let blur_height = small_image.height();
        let url = match format {
            Some(ImageFormat::Png) => {
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
            Some(ImageFormat::Jpeg) => {
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
            Some(ImageFormat::WebP) => {
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
            Some(ImageFormat::Avif) => {
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

        (Some(url), blur_width, blur_height)
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
