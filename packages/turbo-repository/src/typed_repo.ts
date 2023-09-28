// Uses type variables to "reuse" a generic repo, but still produce
// ecosystem-specific information

// Generics

// Generic, any-ecosystem package data
interface Package {
  name: string;
}

interface PackageManager {
  name: string;
}

interface Ecosystem {
  package: Package;
  packageManager: PackageManager;
  name: string;
}

// JS Specific

interface Npm extends PackageManager {
  name: "npm";
}

interface Pnpm extends PackageManager {
  name: "pnpm";
}

type JSPackageManager = Pnpm | Npm;

interface Workspace extends Package {
  // JS-Specific stuff
}

// JS-specific types and constants
export interface JS extends Ecosystem {
  name: "javascript";
  package: Workspace;
  packageManager: JSPackageManager;
}

export class Repository<E extends Ecosystem> {
  readonly ecosystem: string;

  static detectJS(path: string): Repository<JS> {
    throw new Error("not yet implemented");
  }

  private constructor(
    ecosystem: E,
    readonly packageManager: E["packageManager"]
  ) {
    this.ecosystem = ecosystem.name;
  }

  packages(): E["package"][] {
    throw new Error("not yet implemented");
  }
}

const jsRepo = Repository.detectJS("/some/path");
const workspaces: Workspace[] = jsRepo.packages();
const packageManager: JSPackageManager = jsRepo.packageManager;
