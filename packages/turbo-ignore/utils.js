const fs = require("fs");
const path = require("path");
const { findRootSync } = require("@manypkg/find-root");

export function searchUp(pathName, cwd) {
  const root = path.parse(cwd).root;

  let found = false;

  while (!found && cwd !== root) {
    if (fs.existsSync(path.join(cwd, pathName))) {
      found = true;
      break;
    }

    cwd = path.dirname(cwd);
  }

  if (found) {
    return cwd;
  }

  return null;
}

export function getScope() {
  if (process.argv.length > 1 && process.argv[2] != null) {
    return process.argv[2];
  }
  const raw = fs.readFileSync(path.join(process.cwd(), "package.json"), "utf8");
  const pkgJSON = JSON.parse(raw);
  console.log(`≫ Inferred \`${pkgJSON.name}\` as scope from "./package.json"`);
  return pkgJSON.name;
}

export function getRoot() {
  let root = searchUp("turbo.json", process.cwd());

  if (!root) {
    root = findRootSync(process.cwd());
    if (!root) {
      console.error(
        "Error: workspace root not found. turbo-ignore inferencing failed, proceeding with build."
      );
      console.error("");
      process.exit(1);
    }
  }
  return root;
}

export function getComparison() {
  if (process.env.VERCEL === "1") {
    if (process.env.VERCEL_GIT_PREVIOUS_SHA) {
      // use the commit SHA of the last successful deployment for this project / branch
      console.log("\u226B Found previous deployment for project");
      return process.env.VERCEL_GIT_PREVIOUS_SHA;
    } else {
      // this is either the first deploy of the project, or the first deploy for the branch
      // either way - build it.
      console.log(
        `\u226B No previous deployments found for this project on "${process.env.VERCEL_GIT_COMMIT_REF}"`
      );
      console.log(`\u226B Proceeding with build...`);
      process.exit(1);
    }
  }
  return "HEAD^";
}
