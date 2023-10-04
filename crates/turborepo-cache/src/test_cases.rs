use anyhow::Result;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};

pub(crate) struct TestFile {
    path: AnchoredSystemPathBuf,
    contents: &'static str,
}

impl TestFile {
    pub fn file(path: AnchoredSystemPathBuf, contents: &'static str) -> Self {
        Self { path, contents }
    }

    pub fn create(&self, repo_root: &AbsoluteSystemPath) -> Result<()> {
        let file_path = repo_root.resolve(&self.path);
        std::fs::create_dir_all(file_path.parent().unwrap())?;
        std::fs::write(file_path, self.contents)?;

        Ok(())
    }

    pub fn path(&self) -> &AnchoredSystemPath {
        &self.path
    }

    pub fn contents(&self) -> Option<&str> {
        Some(self.contents)
    }
}

pub(crate) struct TestCase {
    pub files: Vec<TestFile>,
    pub duration: u64,
    pub hash: &'static str,
}

impl TestCase {
    pub fn initialize(&self, repo_root: &AbsoluteSystemPath) -> Result<()> {
        for file in &self.files {
            file.create(repo_root)?;
        }

        Ok(())
    }
}

pub(crate) fn get_test_cases() -> Vec<TestCase> {
    vec![
        TestCase {
            files: vec![TestFile::file(
                AnchoredSystemPathBuf::from_raw("package.json").unwrap(),
                "hello world",
            )],
            duration: 58,
            hash: "Faces Places",
        },
        TestCase {
            files: vec![
                TestFile::file(
                    AnchoredSystemPathBuf::from_raw("package.json").unwrap(),
                    "Days of Heaven",
                ),
                TestFile::file(
                    AnchoredSystemPathBuf::from_raw("package-lock.json").unwrap(),
                    "Badlands",
                ),
            ],
            duration: 1284,
            hash: "Cleo from 5 to 7",
        },
        TestCase {
            files: vec![
                TestFile::file(
                    AnchoredSystemPathBuf::from_raw("package.json").unwrap(),
                    "Days of Heaven",
                ),
                TestFile::file(
                    AnchoredSystemPathBuf::from_raw("package-lock.json").unwrap(),
                    "Badlands",
                ),
                TestFile::file(
                    AnchoredSystemPathBuf::from_raw("src/main.js").unwrap(),
                    "Tree of Life",
                ),
            ],
            duration: 12845,
            hash: "The Gleaners and I",
        },
    ]
}
