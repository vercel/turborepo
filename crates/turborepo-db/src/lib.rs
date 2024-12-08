use async_graphql::SimpleObject;
use camino::Utf8Path;
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, SqlitePool};
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::spaces::{
    CreateSpaceRunPayload, RunStatus, SpaceTaskSummary, SpacesCacheStatus,
};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to connect to database: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("failed to migrate database: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("failed to serialize")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, Default, SimpleObject)]
pub struct Run {
    id: String,
    start_time: i64,
    end_time: Option<i64>,
    exit_code: Option<u32>,
    status: String,
    command: String,
    package_inference_root: Option<String>,
    context: String,
    git_branch: Option<String>,
    git_sha: Option<String>,
    origination_user: String,
    client_id: String,
    client_name: String,
    client_version: String,
}

#[derive(Clone)]
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

    pub async fn get_runs(&self, limit: Option<u32>) -> Result<Vec<Run>, Error> {
        let query = if let Some(limit) = limit {
            sqlx::query(
                "SELECT id, start_time, end_time, exit_code, status, command, \
                 package_inference_root, context, git_branch, git_sha, origination_user, \
                 client_id, client_name, client_version FROM runs LIMIT ?",
            )
            .bind(limit)
        } else {
            sqlx::query(
                "SELECT id, start_time, end_time, exit_code, status, command, \
                 package_inference_root, context, git_branch, git_sha, origination_user, \
                 client_id, client_name, client_version FROM runs",
            )
        };

        Ok(query
            .map(|row| Run {
                id: row.get("id"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                exit_code: row.get("exit_code"),
                status: row.get("status"),
                command: row.get("command"),
                package_inference_root: row.get("package_inference_root"),
                context: row.get("context"),
                git_branch: row.get("git_branch"),
                git_sha: row.get("git_sha"),
                origination_user: row.get("origination_user"),
                client_id: row.get("client_id"),
                client_name: row.get("client_name"),
                client_version: row.get("client_version"),
            })
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn get_tasks_for_run(&self, run_id: Uuid) -> Result<Vec<SpaceTaskSummary>, Error> {
        let query = sqlx::query(
            "SELECT key, name, workspace, hash, start_time, end_time, cache_status, exit_code, \
             logs FROM tasks WHERE run_id = ?",
        )
        .bind(run_id.to_string());
        Ok(query
            .map(|row: SqliteRow| SpaceTaskSummary {
                key: row.get("key"),
                name: row.get("name"),
                workspace: row.get("workspace"),
                hash: row.get("hash"),
                start_time: row.get("start_time"),
                end_time: row.get("end_time"),
                cache: SpacesCacheStatus {
                    status: row.get("cache_status"),
                    source: None,
                    time_saved: row.get("time_saved"),
                },
                exit_code: row.get("exit_code"),
                dependencies: row.get("dependencies"),
                dependents: row.get("dependents"),
                logs: row.get("logs"),
            })
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn create_run(&self, payload: &CreateSpaceRunPayload) -> Result<Uuid, Error> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO runs (
              id,
              start_time,
              status,
              command,
              package_inference_root,
              context,
              git_branch,
              git_sha,
              origination_user,
              client_id,
              client_name,
              client_version
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(id.to_string())
        .bind(payload.start_time)
        .bind(payload.status.as_ref())
        .bind(&payload.command)
        .bind(&payload.package_inference_root)
        .bind(payload.run_context)
        .bind(&payload.git_branch)
        .bind(&payload.git_sha)
        .bind(&payload.user)
        .bind(payload.client.id)
        .bind(payload.client.name)
        .bind(&payload.client.version)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    pub async fn finish_run(&self, id: Uuid, end_time: i64, exit_code: i32) -> Result<(), Error> {
        sqlx::query("UPDATE runs SET status = $1, end_time = $2, exit_code = $3 WHERE id = $4")
            .bind(RunStatus::Completed.as_ref())
            .bind(end_time)
            .bind(exit_code)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn finish_task(&self, id: Uuid, summary: &SpaceTaskSummary) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO tasks (
              run_id,
              name,
              package,
              hash,
              start_time,
              end_time,
              cache_status,
              exit_code,
              logs
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id.to_string())
        .bind(&summary.name)
        .bind(&summary.workspace)
        .bind(&summary.hash)
        .bind(summary.start_time)
        .bind(summary.end_time)
        .bind(serde_json::to_string(&summary.cache)?)
        .bind(summary.exit_code)
        .bind(&summary.logs)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use turbopath::AbsoluteSystemPath;
    use turborepo_api_client::spaces::{
        CacheStatus, CreateSpaceRunPayload, RunStatus, SpaceClientSummary, SpaceRunType,
        SpaceTaskSummary, SpacesCacheStatus,
    };

    use crate::DatabaseHandle;

    #[tokio::test]
    async fn test_get_runs() -> Result<(), anyhow::Error> {
        let dir = tempfile::tempdir().unwrap();

        let db = DatabaseHandle::new(
            dir.path().try_into()?,
            AbsoluteSystemPath::from_std_path(dir.path())?,
        )
        .await?;

        let id = db
            .create_run(&CreateSpaceRunPayload {
                start_time: 0,
                status: RunStatus::Running,
                command: "test".to_string(),
                package_inference_root: "test".to_string(),
                run_context: "",
                git_branch: None,
                git_sha: None,
                ty: SpaceRunType::Turbo,
                user: "test".to_string(),
                client: SpaceClientSummary {
                    id: "my-id",
                    name: "turbo",
                    version: "1.0.0".to_string(),
                },
            })
            .await
            .unwrap();
        let runs = db.get_runs(None).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id.len(), 36);
        assert_eq!(runs[0].git_sha, Some("test".to_string()));
        assert_eq!(runs[0].status, "RUNNING".to_string());

        db.finish_task(
            id.clone(),
            &SpaceTaskSummary {
                key: "test#build".to_string(),
                name: "test".to_string(),
                workspace: "test".to_string(),
                hash: "test".to_string(),
                start_time: 0,
                end_time: 0,
                cache: SpacesCacheStatus {
                    status: CacheStatus::Miss,
                    source: None,
                    time_saved: 0,
                },
                exit_code: Some(0),
                dependencies: None,
                dependents: None,
                logs: "".to_string(),
            },
        )
        .await
        .unwrap();

        Ok(())
    }
}
