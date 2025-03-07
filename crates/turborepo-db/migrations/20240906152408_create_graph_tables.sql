CREATE TABLE IF NOT EXISTS package_relations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dependent_package_id INTEGER NOT NULL,
    dependency_package_id INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS task_relations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dependent_task_id INTEGER NOT NULL,
    dependency_task_id INTEGER NOT NULL
);