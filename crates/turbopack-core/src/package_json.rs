use std::fmt::Write;

use anyhow::Result;
use turbo_tasks::primitives::StringVc;
use turbo_tasks_fs::{FileContent, FileJsonContent, FileJsonContentReadRef, FileSystemPathVc};

use super::issue::{Issue, IssueVc};

#[turbo_tasks::value(transparent, serialization = "none")]
pub struct PackageJsonResult(Result<FileJsonContentReadRef, PackageJsonIssueVc>);

#[turbo_tasks::function]
pub async fn read_package_json(path: FileSystemPathVc) -> Result<PackageJsonResultVc> {
    let read = path.read_json().await?;
    match &*read {
        FileJsonContent::Content(_) => Ok(PackageJsonResult(Ok(read)).cell()),
        FileJsonContent::NotFound => Ok(PackageJsonResult(Err(PackageJsonIssue {
            error_message: "package.json file not found".to_string(),
            path,
        }
        .cell()))
        .cell()),
        FileJsonContent::Unparseable(e) => {
            let mut message = "package.json is not parseable: invalid JSON: ".to_string();
            if let FileContent::Content(content) = &*path.read().await? {
                let text = content.content().to_str()?;
                e.write_with_content(&mut message, &text)?;
            } else {
                write!(message, "{}", e)?;
            }
            Ok(PackageJsonResult(Err(PackageJsonIssue {
                error_message: message,
                path,
            }
            .cell()))
            .cell())
        }
    }
}

#[turbo_tasks::value(shared)]
pub struct PackageJsonIssue {
    pub path: FileSystemPathVc,
    pub error_message: String,
}

#[turbo_tasks::value_impl]
impl Issue for PackageJsonIssue {
    #[turbo_tasks::function]
    fn title(&self) -> StringVc {
        StringVc::cell("Error parsing package.json file".to_string())
    }

    #[turbo_tasks::function]
    fn category(&self) -> StringVc {
        StringVc::cell("parse".to_string())
    }

    #[turbo_tasks::function]
    fn context(&self) -> FileSystemPathVc {
        self.path
    }

    #[turbo_tasks::function]
    fn description(&self) -> StringVc {
        StringVc::cell(self.error_message.clone())
    }
}
