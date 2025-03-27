CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    start_time TEXT NOT NULL,
    end_time TEXT,
    exit_code INTEGER,
    command TEXT NOT NULL,
    package_inference_root TEXT,
    git_branch TEXT,
    git_sha TEXT,
    turbo_version TEXT NOT NULL,
    full_turbo BOOLEAN
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
);

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
    start_time TEXT NOT NULL,
    end_time TEXT,
    cache_status TEXT,
    exit_code INTEGER,
    logs TEXT
);
