import { spawnSync } from "node:child_process";
import path from "node:path";
import { rmSync } from "node:fs";

export const DEFAULT_IGNORE = `
# See https://help.github.com/articles/ignoring-files/ for more about ignoring files.

# dependencies
node_modules
.pnp
.pnp.js

# testing
coverage

# misc
.DS_Store
*.pem

# debug
npm-debug.log*
yarn-debug.log*
yarn-error.log*

# turbo
.turbo

# vercel
.vercel
`;

const SHELL_METACHARACTERS = /[`$(){}|;&<>!#]/;

function assertSafeDirectory(dir: string): void {
  if (SHELL_METACHARACTERS.test(dir)) {
    throw new Error(
      `Directory path contains potentially unsafe characters: ${dir}`
    );
  }
}

function git(args: Array<string>, cwd: string): boolean {
  const result = spawnSync("git", args, { stdio: "ignore", cwd });
  if (result.status !== 0) {
    throw new Error(`git ${args[0]} failed`);
  }
  return true;
}

function isInGitRepository(root: string): boolean {
  try {
    git(["rev-parse", "--is-inside-work-tree"], root);
    return true;
  } catch (_) {
    return false;
  }
}

function isInMercurialRepository(root: string): boolean {
  try {
    const result = spawnSync("hg", ["--cwd", ".", "root"], {
      stdio: "ignore",
      cwd: root
    });
    if (result.status !== 0) {
      throw new Error("hg check failed");
    }
    return true;
  } catch (_) {
    return false;
  }
}

/**
 * Initialize a git repository in the given directory with a single commit.
 * This should be called once at the end of the create process, after all
 * files have been created and transforms have been applied.
 *
 * @param root - The absolute path to the directory where the git repository should be initialized
 * @returns true if the repository was initialized successfully, false otherwise
 */
export function tryGitInit(root: string): boolean {
  assertSafeDirectory(root);

  // Skip if already in a git or mercurial repository
  if (isInGitRepository(root) || isInMercurialRepository(root)) {
    return false;
  }

  let didInit = false;
  try {
    git(["init"], root);
    didInit = true;

    git(["checkout", "-b", "main"], root);
    git(["add", "-A"], root);
    git(["commit", "-m", "Initial commit from create-turbo"], root);

    return true;
  } catch (err) {
    if (didInit) {
      try {
        rmSync(path.join(root, ".git"), { recursive: true, force: true });
      } catch (_) {
        // do nothing
      }
    }
    return false;
  }
}

export function removeGitDirectory(root: string): boolean {
  try {
    rmSync(path.join(root, ".git"), { recursive: true, force: true });
    return true;
  } catch (_) {
    return false;
  }
}
