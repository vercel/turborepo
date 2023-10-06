use anyhow::Result;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};

pub(crate) struct TestFile {
    path: AnchoredSystemPathBuf,
    contents: Option<&'static str>,
}

impl TestFile {
    pub fn file(path: AnchoredSystemPathBuf, contents: &'static str) -> Self {
        Self {
            path,
            contents: Some(contents),
        }
    }

    pub fn directory(path: AnchoredSystemPathBuf) -> Self {
        Self {
            path,
            contents: None,
        }
    }

    pub fn create(&self, repo_root: &AbsoluteSystemPath) -> Result<()> {
        let file_path = repo_root.resolve(&self.path);
        match self.contents {
            Some(contents) => {
                std::fs::create_dir_all(file_path.parent().unwrap())?;
                std::fs::write(file_path, contents)?;
            }
            None => {
                std::fs::create_dir(&file_path)?;
            }
        }

        Ok(())
    }

    pub fn path(&self) -> &AnchoredSystemPath {
        &self.path
    }

    pub fn contents(&self) -> Option<&str> {
        self.contents
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
                TestFile::directory(AnchoredSystemPathBuf::from_raw("src").unwrap()),
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
