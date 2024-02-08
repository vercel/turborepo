#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs::{File, OpenOptions},
    io,
    io::{BufRead, Write},
};

use turbopath::AbsoluteSystemPathBuf;

const TURBO_GITIGNORE_ENTRY: &str = ".turbo";
const GITIGNORE_FILE: &str = ".gitignore";

pub fn ensure_turbo_is_gitignored(repo_root: &AbsoluteSystemPathBuf) -> Result<(), io::Error> {
    let gitignore_path = repo_root.join_component(GITIGNORE_FILE);

    if !gitignore_path.exists() {
        let mut gitignore = File::create(gitignore_path)?;
        #[cfg(unix)]
        gitignore.metadata()?.permissions().set_mode(0o0644);
        writeln!(gitignore, "{}", TURBO_GITIGNORE_ENTRY)?;
    } else {
        let gitignore = File::open(&gitignore_path)?;
        let mut lines = io::BufReader::new(gitignore).lines();
        let has_turbo =
            lines.any(|line| line.map_or(false, |line| line.trim() == TURBO_GITIGNORE_ENTRY));
        if !has_turbo {
            let mut gitignore = OpenOptions::new()
                .read(true)
                .append(true)
                .open(&gitignore_path)?;

            // write with a preceding newline just in case the .gitignore file doesn't end
            // with a newline
            writeln!(gitignore, "\n{}", TURBO_GITIGNORE_ENTRY)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;

    use super::*;

    fn has_turbo_gitignore_entry(gitignore_path: &AbsoluteSystemPathBuf) -> bool {
        let gitignore = File::open(gitignore_path).expect("Failed to open .gitignore file");
        let mut lines = io::BufReader::new(gitignore).lines();
        lines.any(|line| line.map_or(false, |line| line.trim() == TURBO_GITIGNORE_ENTRY))
    }

    #[test]
    fn test_no_gitignore() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())?;

        ensure_turbo_is_gitignored(&repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        assert!(has_turbo_gitignore_entry(&gitignore_path));

        Ok(())
    }

    #[test]
    fn gitignore_with_missing_turbo() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())?;

        // create gitignore with no turbo entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        let mut gitignore = File::create(gitignore_path)?;
        #[cfg(unix)]
        gitignore.metadata()?.permissions().set_mode(0o0644);
        writeln!(gitignore, "node_modules/")?;

        ensure_turbo_is_gitignored(&repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        assert!(has_turbo_gitignore_entry(&gitignore_path));

        Ok(())
    }

    #[test]
    fn gitignore_with_existing_turbo() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())?;

        // create gitignore with no turbo entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        let mut gitignore = File::create(gitignore_path)?;
        #[cfg(unix)]
        gitignore.metadata()?.permissions().set_mode(0o0644);
        writeln!(gitignore, ".turbo")?;
        writeln!(gitignore, ".node_modules/")?;

        ensure_turbo_is_gitignored(&repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        assert!(has_turbo_gitignore_entry(&gitignore_path));

        Ok(())
    }

    #[test]
    fn gitignore_with_existing_turbo_no_newline() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root_tmp.path())?;

        // create gitignore with no turbo entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        let mut gitignore = File::create(gitignore_path)?;
        #[cfg(unix)]
        gitignore.metadata()?.permissions().set_mode(0o0644);
        writeln!(gitignore, ".node_modules/")?;
        write!(gitignore, ".turbo")?;

        ensure_turbo_is_gitignored(&repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        assert!(has_turbo_gitignore_entry(&gitignore_path));

        Ok(())
    }
}
