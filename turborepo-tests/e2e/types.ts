type Task = {
  taskId: string;
  task: string;
  package: string;
  hash: string;
  command: string;
  outputs: string[];
  logFile: string;
  directory: string;
  dependencies: string[];
  dependents: string[];
};

export type DryRun = {
  packages: string[];
  tasks: Task[];
};

export type PackageManager = "npm" | "pnpm6" | "pnpm" | "yarn" | "berry";
