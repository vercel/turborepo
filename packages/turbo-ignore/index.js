#!/usr/bin/env node

const { exec } = require("child_process");
const fs = require("fs");
const path = require("path");
const { findRootSync } = require("@manypkg/find-root");

console.log(
  "≫ Using Turborepo to determine if this project is affected by the commit..."
);
const root = getRoot();
const scope = getScope();
const command = `npx turbo run build --filter=${scope}...[HEAD^] --dry=json`;
console.log(`≫ Analyzing results of \`${command}\`...`);
exec(
  command,
  {
    cwd: root,
  },
  (error, stdout, stderr) => {
    if (error) {
      console.error(`exec error: ${error}`);
      return;
    }

    try {
      const parsed = JSON.parse(stdout);
      if (parsed == null) {
        console.error(`≫ Failed to parse JSON output from \`${command}\`.`);
        console.error(`≫ Proceeding with build to be safe...`);
        process.exit(1);
      }
      const { packages } = parsed;
      if (packages && packages.length > 0) {
        console.log(
          `≫ The commit affects this project and/or its ${
            packages.length - 1
          } dependencies`
        );
        console.log(`≫ Proceeding with build...`);
        process.exit(1);
      } else {
        console.log("≫ This project and its dependencies are not affected");
        console.log("≫ Ignoring the change");
        process.exit(0);
      }
    } catch (e) {
      console.error(`≫ Failed to parse JSON output from \`${command}\`.`);
      console.error(e);
      console.error(`≫ Proceeding with build to be safe...`);
      process.exit(1);
    }
  }
);

function searchUp(pathName, cwd) {
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

function getScope() {
  if (process.argv.length > 1 && process.argv[2] != null) {
    return process.argv[2];
  }
  const raw = fs.readFileSync(path.join(process.cwd(), "package.json"), "utf8");
  const pkgJSON = JSON.parse(raw);
  console.log(`≫ Inferred \`${pkgJSON.name}\` as scope from "./package.json"`);
  return pkgJSON.name;
}

function getRoot() {
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
