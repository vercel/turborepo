import { execSync } from "node:child_process";
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

export const GIT_REPO_COMMAND = "git rev-parse --is-inside-work-tree";
export const HG_REPO_COMMAND = "hg --cwd . root";

function isInGitRepository(root: string): boolean {
  try {
    execSync(GIT_REPO_COMMAND, { stdio: "ignore", cwd: root });
    return true;
  } catch (_) {
    return false;
  }
}

function isInMercurialRepository(root: string): boolean {
  try {
    execSync(HG_REPO_COMMAND, { stdio: "ignore", cwd: root });
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
  // Skip if already in a git or mercurial repository
  if (isInGitRepository(root) || isInMercurialRepository(root)) {
    return false;
  }

  let didInit = false;
  try {
    execSync("git init", { stdio: "ignore", cwd: root });
    didInit = true;

    execSync("git checkout -b main", { stdio: "ignore", cwd: root });
    execSync("git add -A", { stdio: "ignore", cwd: root });
    execSync('git commit -m "Initial commit from create-turbo"', {
      stdio: "ignore",
      cwd: root,
    });

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
