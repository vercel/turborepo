use std::{
    fs::{File, OpenOptions},
    io,
    io::{BufRead, Write},
};

use turbopath::AbsoluteSystemPath;

const TURBO_GITIGNORE_COMMENT: &str = "# Turborepo";
const TURBO_GITIGNORE_ENTRY: &str = ".turbo";
const GITIGNORE_FILE: &str = ".gitignore";

fn has_turbo_gitignore_entry(mut lines: io::Lines<io::BufReader<File>>) -> bool {
    lines.any(|line| line.map_or(false, |line| line.trim() == TURBO_GITIGNORE_ENTRY))
}

fn get_ignore_string() -> String {
    format!("{}\n{}", TURBO_GITIGNORE_COMMENT, TURBO_GITIGNORE_ENTRY)
}

pub fn ensure_turbo_is_gitignored(repo_root: &AbsoluteSystemPath) -> Result<(), io::Error> {
    let gitignore_path = repo_root.join_component(GITIGNORE_FILE);

    if !gitignore_path.try_exists().unwrap_or(true) {
        gitignore_path.create_with_contents(get_ignore_string())?;
        #[cfg(unix)]
        {
            gitignore_path.set_mode(0o0644)?;
        }
    } else {
        let gitignore = gitignore_path.open()?;
        let lines = io::BufReader::new(gitignore).lines();
        let has_turbo = has_turbo_gitignore_entry(lines);
        if !has_turbo {
            let mut opts = OpenOptions::new();
            opts.read(true).write(true).append(true);
            let mut gitignore = &gitignore_path.open_with_options(opts)?;

            // write with a preceding newline just in case the .gitignore file doesn't end
            // with a newline
            writeln!(gitignore, "\n{}", get_ignore_string())?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;

    use super::*;

    fn check_for_turbo_in_gitignore_file(gitignore_path: &AbsoluteSystemPath) -> bool {
        let gitignore = gitignore_path
            .open()
            .expect("Failed to open .gitignore file");

        let lines = io::BufReader::new(gitignore).lines();
        has_turbo_gitignore_entry(lines)
    }

    fn get_gitignore_size(gitignore_path: &AbsoluteSystemPath) -> usize {
        let gitignore = gitignore_path
            .open()
            .expect("Failed to open .gitignore file");

        let lines = io::BufReader::new(gitignore).lines();
        lines.count()
    }

    #[test]
    fn test_no_gitignore() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(repo_root_tmp.path())?;

        ensure_turbo_is_gitignored(repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        /* Expected .gitignore file contents:
        1. # Turborepo
        2. .turbo
         */
        assert!(check_for_turbo_in_gitignore_file(&gitignore_path));
        assert_eq!(get_gitignore_size(&gitignore_path), 2);
        Ok(())
    }

    #[test]
    fn gitignore_with_missing_turbo() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(repo_root_tmp.path())?;

        // create gitignore with no turbo entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        gitignore_path.create_with_contents("node_modules/\n")?;
        #[cfg(unix)]
        {
            gitignore_path.set_mode(0o0644)?;
        }
        assert_eq!(get_gitignore_size(&gitignore_path), 1);

        ensure_turbo_is_gitignored(repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        /* Expected .gitignore file contents:
        1. node_modules/
        2.
        3. # Turborepo
        4. .turbo
         */
        assert!(check_for_turbo_in_gitignore_file(&gitignore_path));
        assert_eq!(get_gitignore_size(&gitignore_path), 4);

        Ok(())
    }

    #[test]
    fn gitignore_with_existing_turbo_without_comment() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(repo_root_tmp.path())?;

        // create gitignore with no turbo entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        gitignore_path.create_with_contents("node_modules/\n.turbo\n")?;
        assert_eq!(get_gitignore_size(&gitignore_path), 2);

        ensure_turbo_is_gitignored(repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        /* Expected .gitignore file contents:
        1. node_modules/
        2..turbo/
         */
        assert!(check_for_turbo_in_gitignore_file(&gitignore_path));
        assert_eq!(get_gitignore_size(&gitignore_path), 2);

        Ok(())
    }

    #[test]
    fn gitignore_with_existing_turbo_with_comment() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(repo_root_tmp.path())?;

        // create gitignore with no turbo entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        gitignore_path.create_with_contents("node_modules/\n# Turborepo\n.turbo")?;
        assert_eq!(get_gitignore_size(&gitignore_path), 3);

        ensure_turbo_is_gitignored(repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        /* Expected .gitignore file contents:
        1. node_modules/
        2. # Turborepo
        3..turbo/
         */
        assert!(check_for_turbo_in_gitignore_file(&gitignore_path));
        assert_eq!(get_gitignore_size(&gitignore_path), 3);

        Ok(())
    }

    #[test]
    fn gitignore_with_missing_turbo_no_newline() -> Result<()> {
        let repo_root_tmp = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(repo_root_tmp.path())?;

        // create gitignore with no turbo entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        gitignore_path.create_with_contents("node_modules/")?;
        assert_eq!(get_gitignore_size(&gitignore_path), 1);

        ensure_turbo_is_gitignored(repo_root).expect("Failed to ensure turbo is gitignored");

        // Verify that the .gitignore file exists and contains the expected entry
        let gitignore_path = repo_root.join_component(GITIGNORE_FILE);
        assert!(gitignore_path.exists());

        /* Expected .gitignore file contents:
        1. node_modules/
        2. # Turborepo
        3. .turbo
         */
        assert!(check_for_turbo_in_gitignore_file(&gitignore_path));
        assert_eq!(get_gitignore_size(&gitignore_path), 3);

        Ok(())
    }
}
