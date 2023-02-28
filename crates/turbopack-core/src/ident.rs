use std::fmt::Write;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbo_tasks::{
    primitives::StringVc, trace::TraceRawVcs, Value, ValueToString, ValueToStringVc,
};
use turbo_tasks_fs::FileSystemPathVc;

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TraceRawVcs,
)]
pub enum AssetParam {
    Query(StringVc),
    Fragment(StringVc),
    Asset(StringVc, AssetIdentVc),
    Modifier(StringVc),
}

#[turbo_tasks::value(serialization = "auto_for_input")]
#[derive(Clone, Debug, PartialOrd, Ord, Hash)]
pub struct AssetIdent {
    pub path: FileSystemPathVc,
    /// The parameters of the [AssetIdent]. They must be in the order they
    /// appear in the enum: Query -> Fragment -> Asset -> Modifier. Any other
    /// order is considered as invalid.
    /// Assets in the output level must not have any parameters.
    pub params: Vec<AssetParam>,
}

impl AssetIdent {
    pub fn add_modifier(&mut self, modifier: StringVc) {
        self.params.push(AssetParam::Modifier(modifier));
    }

    pub fn add_asset(&mut self, key: StringVc, asset: AssetIdentVc) {
        // insert into correct position
        let index = self
            .params
            .iter()
            .rposition(|param| matches!(param, AssetParam::Asset(..) | AssetParam::Modifier(..)))
            .map_or(0, |x| x + 1);
        self.params.insert(index, AssetParam::Asset(key, asset));
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for AssetIdent {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        let mut s = self.path.to_string().await?.clone_value();
        let mut has_modifier = false;
        for param in &self.params {
            match param {
                AssetParam::Query(query) => {
                    s.push('?');
                    s.push_str(&query.await?);
                }
                AssetParam::Fragment(fragment) => {
                    s.push('#');
                    s.push_str(&fragment.await?);
                }
                AssetParam::Asset(key, asset) => {
                    write!(s, "/({})/{}", key.await?, asset.to_string().await?)?;
                }
                AssetParam::Modifier(key) => {
                    if has_modifier {
                        s.pop();
                        write!(s, ", {})", key.await?)?;
                    } else {
                        write!(s, " ({})", key.await?)?;
                        has_modifier = true;
                    }
                }
            }
        }
        Ok(StringVc::cell(s))
    }
}

#[turbo_tasks::value_impl]
impl AssetIdentVc {
    #[turbo_tasks::function]
    pub fn new(ident: Value<AssetIdent>) -> Self {
        ident.into_value().cell()
    }

    /// Creates an [AssetIdent] from a [FileSystemPathVc]
    #[turbo_tasks::function]
    pub fn from_path(path: FileSystemPathVc) -> Self {
        Self::new(Value::new(AssetIdent {
            path,
            params: Vec::new(),
        }))
    }

    #[turbo_tasks::function]
    pub async fn with_modifier(self, modifier: StringVc) -> Result<Self> {
        let mut this = self.await?.clone_value();
        this.params.push(AssetParam::Modifier(modifier));
        Ok(Self::new(Value::new(this)))
    }

    #[turbo_tasks::function]
    pub async fn path(self) -> Result<FileSystemPathVc> {
        Ok(self.await?.path)
    }
}
