// Fully generic. The initial constructor informs how to resolve
// the repository, but nothing in the resulting types is js-specific

import type { ChildProcess } from "child_process";

export class Lockfile {
  // TODO: what interesting lockfile details should we expose?
}

export class Workspace {
  private constructor(readonly name: string, readonly path: string) {}
}

export class PackageManager {
  private constructor(
    readonly name: string,
    readonly version: string,
    readonly bin: string
  ) {}

  exec(cmd: string, args: string[]): ChildProcess {
    throw new Error("unimplemented");
  }

  // Method due to needing IO, should maybe be async?
  lockfile(): Lockfile {
    throw new Error("unimplemented");
  }
}

export class Repository {
  static detectJS(path: string): Repository {
    throw new Error("unimplemented");
  }

  readonly isMonorepo: boolean;

  private constructor(
    readonly root: string,
    readonly packageManager: PackageManager
  ) {
    throw new Error("Not yet implemented");
  }

  workspaces(): Workspace[] {
    throw new Error("Not yet implemented");
  }
}
