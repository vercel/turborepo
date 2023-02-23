use crate::{anchored_unix_tar_path_buf::AnchoredUnixTarPathBuf, RelativeUnixPathBuf};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AnchoredUnixPathBuf(pub(crate) String);

impl AnchoredUnixPathBuf {
    pub fn into_inner(self) -> String {
        self.0
    }

    pub fn make_canonical_for_tar(mut self, is_dir: bool) -> AnchoredUnixTarPathBuf {
        if is_dir {
            if !self.0.ends_with("/") {
                self.0.push('/');
            }
        }

        AnchoredUnixTarPathBuf(self.0)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Into<RelativeUnixPathBuf> for AnchoredUnixPathBuf {
    fn into(self) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf::new_unchecked(self.0)
    }
}

impl From<RelativeUnixPathBuf> for AnchoredUnixPathBuf {
    fn from(path: RelativeUnixPathBuf) -> Self {
        AnchoredUnixPathBuf(path.into_inner())
    }
}
