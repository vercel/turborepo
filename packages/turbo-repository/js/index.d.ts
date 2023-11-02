export class Repository {
  readonly root: string;
  readonly isMonorepo: boolean;
  static detectJS(path?: string | undefined | null): Repository;
  packageManager(): PackageManager;
  workspaces(): Promise<Array<Workspace>>;
}
export class PackageManager {
  readonly name: string;
}
export class Workspace {
  readonly absolutePath: string;
  readonly repoPath: string;
}
