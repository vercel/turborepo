CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    start_time BIGINT NOT NULL,
    end_time BIGINT,
    exit_code INTEGER,
    status TEXT NOT NULL,
    command TEXT NOT NULL,
    package_inference_root TEXT,
    context TEXT NOT NULL,
    git_branch TEXT,
    git_sha TEXT,
    origination_user TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_name TEXT NOT NULL,
    client_version TEXT NOT NULL
    full_turbo BOOLEAN NOT NULL
);

CREATE TABLE IF NOT EXISTS config (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    api_url TEXT NOT NULL,
    login_url TEXT NOT NULL,
    team_slug TEXT NOT NULL,
    team_id TEXT NOT NULL,
    signature BOOLEAN NOT NULL,
    preflight BOOLEAN NOT NULL,
    timeout INTEGER NOT NULL,
    upload_timeout INTEGER NOT NULL,
    enabled BOOLEAN NOT NULL,
    spaces_id TEXT NOT NULL,
    global_dependencies TEXT NOT NULL,
    global_env TEXT NOT NULL,
    global_pass_through_env TEXT NOT NULL,
    tasks TEXT NOT NULL,
    cache_dir TEXT NOT NULL,
    root_turbo_json TEXT NOT NULL
)

CREATE TABLE IF NOT EXISTS packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    path TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS package_dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dependent_id INTEGER NOT NULL,
    dependency_id INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    name TEXT NOT NULL,
    hash TEXT NOT NULL,
    package_id INTEGER NOT NULL,
    start_time INTEGER NOT NULL,
    end_time INTEGER NOT NULL,
    cache_status TEXT NOT NULL
    exit_code INTEGER,
    logs TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task_dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dependent_id INTEGER NOT NULL,
    dependency_id INTEGER NOT NULL
);