use crate::{Error, Git};

pub enum CloneMode {
    /// Cloning locally, do a blobless clone (good UX and reasonably fast)
    Local,
    /// Cloning on CI, do a treeless clone (worse UX but fastest)
    CI,
}

// Provide a sane maximum depth for cloning. If a user needs more history than
// this, they can override or fetch it themselves.
const MAX_CLONE_DEPTH: usize = 64;

impl Git {
    pub fn clone(
        &self,
        url: &str,
        dir: Option<&str>,
        branch: Option<&str>,
        mode: CloneMode,
        depth: Option<usize>,
    ) -> Result<(), Error> {
        let depth = depth.unwrap_or(MAX_CLONE_DEPTH).to_string();
        let mut args = vec!["clone", "--depth", &depth];
        if let Some(branch) = branch {
            args.push("--branch");
            args.push(branch);
        }
        match mode {
            CloneMode::Local => {
                args.push("--filter=blob:none");
            }
            CloneMode::CI => {
                args.push("--filter=tree:0");
            }
        }
        args.push(url);
        if let Some(dir) = dir {
            args.push(dir);
        }

        self.spawn_git_command(&args, "")?;

        Ok(())
    }
}
