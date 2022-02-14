/* eslint-disable import/no-extraneous-dependencies */
import { execSync } from "child_process";
import { rmdirSync } from "fs"
import path from "path";

function isInGitRepository(): boolean {
  try {
    execSync("git rev-parse --is-inside-work-tree", { stdio: "ignore" });
    return true;
  } catch (_) {}
  return false;
}

function isInMercurialRepository(): boolean {
  try {
    execSync("hg --cwd . root", { stdio: "ignore" });
    return true;
  } catch (_) {}
  return false;
}

export function tryGitInit(root: string): boolean {
  let didInit = false;
  try {
    execSync("git --version", { stdio: "ignore" });
    if (isInGitRepository() || isInMercurialRepository()) {
      return false;
    }

    execSync("git init", { stdio: "ignore" });
    didInit = true;

    execSync("git checkout -b main", { stdio: "ignore" });

    execSync("git add -A", { stdio: "ignore" });
    execSync('git commit -m "Initial commit from Create Turborepo"', {
      stdio: "ignore",
    });
    return true;
  } catch (e) {
    if (didInit) {
      try {
        rmdirSync(path.join(root, ".git"), { recursive:true });
      } catch (_) {}
    }
    return false;
  }
}
