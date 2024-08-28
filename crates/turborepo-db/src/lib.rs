#![feature(assert_matches)]

use camino::Utf8Path;
use chrono::{DateTime, Utc};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, SqlitePool};
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("failed to migrate database: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("failed to serialize")]
    Serialize(#[from] serde_json::Error),
    #[error("invalid cache status: {status}")]
    InvalidCacheStatus { status: String },
}

#[derive(Debug, Default)]
pub struct Run {
    id: u32,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    exit_code: Option<u32>,
    command: String,
    package_inference_root: Option<String>,
    git_branch: Option<String>,
    git_sha: Option<String>,
    turbo_version: String,
    full_turbo: Option<bool>,
}

#[derive(Debug, Default)]
pub struct StartRunPayload {
    start_time: DateTime<Utc>,
    command: String,
    package_inference_root: Option<String>,
    git_branch: Option<String>,
    git_sha: Option<String>,
    turbo_version: String,
}

pub struct FinishRunPayload {
    end_time: DateTime<Utc>,
    exit_code: i32,
    full_turbo: bool,
}

pub struct Task {
    id: u32,
    run_id: u32,
    name: String,
    hash: String,
    package: String,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    cache_status: Option<CacheStatus>,
    exit_code: Option<i32>,
    output: Option<String>,
}

pub struct StartTaskPayload {
    run_id: u32,
    name: String,
    hash: String,
    package: String,
    package_path: AnchoredSystemPathBuf,
    start_time: DateTime<Utc>,
}

pub struct FinishTaskPayload {
    end_time: DateTime<Utc>,
    cache_status: CacheStatus,
    exit_code: i32,
    logs: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheStatus {
    Miss,
    Hit,
}

impl AsRef<str> for CacheStatus {
    fn as_ref(&self) -> &str {
        match self {
            CacheStatus::Miss => "MISS",
            CacheStatus::Hit => "HIT",
        }
    }
}

enum RunStatus {
    Running,
    Completed,
}

impl AsRef<str> for RunStatus {
    fn as_ref(&self) -> &str {
        match self {
            RunStatus::Running => "RUNNING",
            RunStatus::Completed => "COMPLETED",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseHandle {
    pool: Pool<Sqlite>,
}

impl DatabaseHandle {
    pub async fn new(cache_dir: &Utf8Path, repo_root: &AbsoluteSystemPath) -> Result<Self, Error> {
        let cache_dir = AbsoluteSystemPathBuf::from_unknown(&repo_root, &cache_dir);
        let pool = SqlitePool::connect(&format!(
            "sqlite://{}?mode=rwc",
            cache_dir.join_component("turbo.db")
        ))
        .await?;

        sqlx::migrate!().run(&pool).await?;

        Ok(Self { pool })
    }

    pub async fn get_run(&self, id: u32) -> Result<Option<Run>, Error> {
        let query = sqlx::query(
            "SELECT id, start_time, end_time, exit_code, command, package_inference_root, \
             git_branch, git_sha, turbo_version, full_turbo FROM runs WHERE id = $1",
        )
        .bind(id);

        Ok(query
            .map(|row| Run {
                id: row.get("id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                exit_code: row.get("exit_code"),
                command: row.get("command"),
                package_inference_root: row.get("package_inference_root"),
                git_branch: row.get("git_branch"),
                git_sha: row.get("git_sha"),
                turbo_version: row.get("turbo_version"),
                full_turbo: row.get("full_turbo"),
            })
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn get_runs(&self, limit: Option<u32>) -> Result<Vec<Run>, Error> {
        let query = if let Some(limit) = limit {
            sqlx::query(
                "SELECT id, start_time, end_time, exit_code, command, package_inference_root, \
                 git_branch, git_sha, turbo_version, full_turbo FROM runs LIMIT ?",
            )
            .bind(limit)
        } else {
            sqlx::query(
                "SELECT id, start_time, end_time, exit_code, command, package_inference_root, \
                 git_branch, git_sha, turbo_version, full_turbo FROM runs",
            )
        };

        Ok(query
            .map(|row| Run {
                id: row.get("id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                exit_code: row.get("exit_code"),
                command: row.get("command"),
                package_inference_root: row.get("package_inference_root"),
                git_branch: row.get("git_branch"),
                git_sha: row.get("git_sha"),
                turbo_version: row.get("turbo_version"),
                full_turbo: row.get("full_turbo"),
            })
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn get_tasks_for_run(&self, run_id: u32) -> Result<Vec<Task>, Error> {
        let query = sqlx::query(
            "SELECT tasks.id, tasks.name AS task_name, packages.name AS package_name, hash, \
             start_time, end_time, cache_status, exit_code, logs FROM tasks JOIN packages ON \
             tasks.package_id = packages.id WHERE run_id = $1",
        )
        .bind(run_id);

        Ok(query
            .try_map(|row: SqliteRow| {
                Ok(Task {
                    id: row.get("id"),
                    run_id,
                    name: row.get("task_name"),
                    package: row.get("package_name"),
                    hash: row.get("hash"),
                    start_time: row.get("start_time"),
                    end_time: row.get("end_time"),
                    cache_status: match row.get("cache_status") {
                        Some("MISS") => Some(CacheStatus::Miss),
                        Some("HIT") => Some(CacheStatus::Hit),
                        Some(status) => {
                            return Err(sqlx::Error::Decode(Box::new(Error::InvalidCacheStatus {
                                status: status.to_string(),
                            })))
                        }
                        None => None,
                    },
                    exit_code: row.get("exit_code"),
                    output: None,
                })
            })
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn start_task(&self, task: &StartTaskPayload) -> Result<u32, Error> {
        let package_row = sqlx::query("SELECT id FROM packages WHERE name = $1")
            .bind(&task.package)
            .fetch_optional(&self.pool)
            .await?;

        let package_id: u32 = if let Some(package_row) = package_row {
            package_row.get("id")
        } else {
            sqlx::query("INSERT INTO packages (name, path) VALUES ($1, $2) RETURNING id")
                .bind(&task.package)
                .bind(task.package_path.as_str())
                .fetch_one(&self.pool)
                .await?
                .get("id")
        };

        let row = sqlx::query(
            "INSERT INTO tasks (
              run_id,
              name,
              package_id,
              hash,
              start_time
            ) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(task.run_id)
        .bind(&task.name)
        .bind(package_id)
        .bind(&task.hash)
        .bind(&task.start_time)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("id"))
    }

    pub async fn start_run(&self, run: &StartRunPayload) -> Result<u32, Error> {
        let row = sqlx::query(
            "INSERT INTO runs (
              start_time,
              command,
              package_inference_root,
              git_branch,
              git_sha,
              turbo_version
             ) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(&run.start_time)
        .bind(&run.command)
        .bind(&run.package_inference_root)
        .bind(&run.git_branch)
        .bind(&run.git_sha)
        .bind(&run.turbo_version)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("id"))
    }

    pub async fn finish_run(&self, id: u32, payload: &FinishRunPayload) -> Result<(), Error> {
        sqlx::query("UPDATE runs SET end_time = $1, exit_code = $2, full_turbo = $3 WHERE id = $4")
            .bind(&payload.end_time)
            .bind(&payload.exit_code)
            .bind(&payload.full_turbo)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn finish_task(&self, id: u32, payload: &FinishTaskPayload) -> Result<(), Error> {
        sqlx::query(
            "UPDATE tasks SET
              end_time = $1,
              cache_status = $2,
              exit_code = $3,
              logs = $4
            WHERE id = $5",
        )
        .bind(payload.end_time)
        .bind(&payload.cache_status.as_ref())
        .bind(payload.exit_code)
        .bind(&payload.logs)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::assert_matches::assert_matches;

    use chrono::Utc;
    use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

    use crate::{
        CacheStatus, DatabaseHandle, FinishRunPayload, FinishTaskPayload, StartRunPayload,
        StartTaskPayload,
    };

    #[tokio::test]
    async fn test_persistence() -> Result<(), anyhow::Error> {
        let dir = tempfile::tempdir().unwrap();

        let db = DatabaseHandle::new(
            dir.path().try_into()?,
            AbsoluteSystemPath::from_std_path(dir.path())?,
        )
        .await?;

        let run_id = db
            .start_run(&StartRunPayload {
                start_time: Utc::now(),
                command: "test".to_string(),
                package_inference_root: Some("foo/bar".to_string()),
                git_branch: None,
                git_sha: Some("my-sha".to_string()),
                turbo_version: "".to_string(),
            })
            .await?;

        let runs = db.get_runs(None).await?;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].git_sha, Some("my-sha".to_string()));
        assert_eq!(runs[0].package_inference_root, Some("foo/bar".to_string()));
        assert_eq!(runs[0].end_time, None);

        db.start_task(&StartTaskPayload {
            run_id,
            name: "test".to_string(),
            package: "test".to_string(),
            package_path: AnchoredSystemPathBuf::from_raw("packages/test")?,
            hash: "test".to_string(),
            start_time: Utc::now(),
        })
        .await?;

        let tasks = db.get_tasks_for_run(run_id).await?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "test");

        db.finish_task(
            tasks[0].id,
            &FinishTaskPayload {
                end_time: Utc::now(),
                cache_status: CacheStatus::Hit,
                exit_code: 0,
                logs: "test".to_string(),
            },
        )
        .await?;

        let tasks = db.get_tasks_for_run(run_id).await?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "test");
        assert_matches!(tasks[0].cache_status, Some(CacheStatus::Hit));
        assert_eq!(tasks[0].exit_code, Some(0));

        db.finish_run(
            run_id,
            &FinishRunPayload {
                end_time: Utc::now(),
                exit_code: 0,
                full_turbo: true,
            },
        )
        .await?;

        let run = db.get_run(run_id).await?.unwrap();
        assert!(run.end_time.is_some());
        assert_eq!(run.exit_code, Some(0));
        assert!(run.full_turbo.unwrap());

        Ok(())
    }
}
