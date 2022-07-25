#!/usr/bin/env node

const { exec } = require("child_process");
import { getRoot, getScope, getComparison } from "./utils";

console.log(
  "≫ Using Turborepo to determine if this project is affected by the commit..."
);
const root = getRoot();
const scope = getScope();
const comparison = getComparison();
const command = `npx turbo run build --filter=${scope}...[${comparison}] --dry=json`;
console.log(`≫ Analyzing results of \`${command}\`...`);
exec(
  command,
  {
    cwd: root,
  },
  (error, stdout, stderr) => {
    if (error) {
      console.error(`exec error: ${error}`);
      console.error(`≫ Proceeding with build to be safe...`);
      process.exit(1);
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
