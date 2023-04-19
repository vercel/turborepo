use std::io::Write;

use anyhow::{bail, Result};
use turbo_tasks::primitives::StringVc;
use turbo_tasks_fs::{rope::RopeBuilder, FileContent};
use turbopack_core::{
    asset::{Asset, AssetContent, AssetContentVc, AssetVc},
    ident::AssetIdentVc,
};
use turbopack_ecmascript::utils::StringifyJs;

use crate::process::get_meta_data_and_blur_placeholder;

fn modifier() -> StringVc {
    StringVc::cell("structured image object".to_string())
}

#[turbo_tasks::value(shared)]
pub struct StructuredImageSourceAsset {
    pub image: AssetVc,
}

#[turbo_tasks::value_impl]
impl Asset for StructuredImageSourceAsset {
    #[turbo_tasks::function]
    fn ident(&self) -> AssetIdentVc {
        self.image.ident().with_modifier(modifier())
    }

    #[turbo_tasks::function]
    async fn content(&self) -> Result<AssetContentVc> {
        let content = self.image.content().await?;
        let AssetContent::File(content) = *content else {
            bail!("Input source is not a file and can't be transformed into image information");
        };
        let mut result = RopeBuilder::from("");
        let info = get_meta_data_and_blur_placeholder(self.image.ident(), content);
        let info = info.await?;
        writeln!(result, "import src from \"IMAGE\";",)?;
        writeln!(
            result,
            "export default {{ src, ...{} }}",
            StringifyJs(&*info)
        )?;
        Ok(AssetContent::File(FileContent::Content(result.build().into()).cell()).cell())
    }
}
