use crate::{Error, Git};

pub enum CloneMode {
    /// Cloning locally, do a blobless clone (good UX and reasonably fast)
    Local,
    /// Cloning on CI, do a treeless clone (worse UX but fastest)
    CI,
}

impl Git {
    pub fn clone(
        &self,
        url: &str,
        dir: Option<&str>,
        branch: Option<&str>,
        mode: CloneMode,
    ) -> Result<(), Error> {
        let mut args = vec!["clone"];
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
