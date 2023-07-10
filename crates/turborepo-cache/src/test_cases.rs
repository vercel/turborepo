use anyhow::Result;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

pub(crate) struct TestFile {
    pub path: AnchoredSystemPathBuf,
    pub contents: &'static str,
}

impl TestFile {
    pub fn create(&self, repo_root: &AbsoluteSystemPath) -> Result<()> {
        let file_path = repo_root.resolve(&self.path);
        std::fs::create_dir_all(file_path.parent().unwrap())?;
        std::fs::write(file_path, &self.contents)?;

        Ok(())
    }
}

pub(crate) struct TestCase {
    pub files: Vec<TestFile>,
    pub duration: u32,
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
            files: vec![TestFile {
                path: AnchoredSystemPathBuf::from_raw("package.json").unwrap(),
                contents: "hello world",
            }],
            duration: 58,
            hash: "Faces Places",
        },
        TestCase {
            files: vec![
                TestFile {
                    path: AnchoredSystemPathBuf::from_raw("package.json").unwrap(),
                    contents: "Days of Heaven",
                },
                TestFile {
                    path: AnchoredSystemPathBuf::from_raw("package-lock.json").unwrap(),
                    contents: "Badlands",
                },
            ],
            duration: 1284,
            hash: "Cleo from 5 to 7",
        },
        TestCase {
            files: vec![
                TestFile {
                    path: AnchoredSystemPathBuf::from_raw("package.json").unwrap(),
                    contents: "Days of Heaven",
                },
                TestFile {
                    path: AnchoredSystemPathBuf::from_raw("package-lock.json").unwrap(),
                    contents: "Badlands",
                },
                TestFile {
                    path: AnchoredSystemPathBuf::from_raw("src/main.js").unwrap(),
                    contents: "Tree of Life",
                },
            ],
            duration: 12845,
            hash: "The Gleaners and I",
        },
    ]
}
