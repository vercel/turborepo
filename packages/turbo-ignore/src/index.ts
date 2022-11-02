#!/usr/bin/env node
import meow from "meow";
import { exec } from "child_process";
import { getTurboRoot, getScopeFromPath, getScopeFromArgs } from "turbo-utils";
import { getComparison } from "./getComparison";

run()
  .then(() => {})
  .catch(() => {
    process.exit(1);
  });

const helpText = `
  Usage:
    $ npx turbo-ignore

  Flags:
    --help, -h          Show this help message
    --version, -v       Show the version of this script
`;

async function run() {
  let { flags, input, showHelp, showVersion } = meow(helpText, {
    flags: {
      help: { type: "boolean", default: false, alias: "h" },
    },
  });

  // these helper functions will process.exit()
  if (flags.help) showHelp();
  if (flags.version) showVersion();

  console.log(
    "\u226B Using Turborepo to determine if this project is affected by the commit..."
  );

  // check for TURBO_FORCE and bail early if it's set
  if (process.env.TURBO_FORCE === "true") {
    console.log(
      "\u226B `TURBO_FORCE` detected, skipping check and proceeding with build."
    );
    return Promise.reject();
  }

  // find the monorepo root
  const root = getTurboRoot();
  if (!root) {
    console.error(
      "Error: workspace root not found. turbo-ignore inferencing failed, proceeding with build."
    );
    console.error("");
    return Promise.reject();
  }

  // Find the scope of the project

  const argsScope = getScopeFromArgs({ args: process.argv.slice(2) });
  const pathScope = getScopeFromPath({ cwd: process.cwd() });
  const { context, scope } = argsScope.scope ? argsScope : pathScope;
  if (!scope) {
    console.error(
      "Error: app scope not found. turbo-ignore inferencing failed, proceeding with build."
    );
    if (!pathScope.scope) {
      console.error(
        'Error: the package.json is missing the "name" field.\nSet this field or pass the scope name as the first argument to turbo-ignore.'
      );
    }
    console.error("");
    return Promise.reject();
  }

  if (context.path) {
    console.log(`\u226B Inferred \`${scope}\` as scope from "${context.path}"`);
  } else {
    console.log(`\u226B Inferred \`${scope}\` as scope from arguments`);
  }

  // Get the start of the comparison (previous deployment when available, or previous commit by default)
  const comparison = getComparison();
  if (!comparison) {
    // This is either the first deploy of the project, or the first deploy for the branch, either way - build it.
    console.log(
      `\u226B No previous deployments found for this project${
        process.env.VERCEL === "1"
          ? ` on "${process.env.VERCEL_GIT_COMMIT_REF}.`
          : "."
      }"`
    );
    console.log(`\u226B Proceeding with build...`);
    return Promise.reject();
  }

  if (comparison.type === "previousDeploy") {
    console.log("\u226B Found previous deployment for project");
  }

  // Build, and execute the command
  const command = `npx turbo run build --filter=${scope}...[${comparison.ref}] --dry=json`;
  console.log(`\u226B Analyzing results of \`${command}\`...`);

  return new Promise((resolve, reject) => {
    exec(
      command,
      {
        cwd: root,
      },
      (error, stdout) => {
        if (error) {
          console.error(`exec error: ${error}`);
          console.error(`\u226B Proceeding with build to be safe...`);
          process.exit(1);
        }

        try {
          const parsed = JSON.parse(stdout);
          if (parsed == null) {
            console.error(
              `\u226B Failed to parse JSON output from \`${command}\`.`
            );
            console.error(`\u226B Proceeding with build to be safe...`);
            reject();
            return;
          }

          const { packages } = parsed;

          if (packages && packages.length > 0) {
            console.log(
              `\u226B The commit affects this project and/or its ${
                packages.length - 1
              } dependencies`
            );
            console.log(`\u226B Proceeding with build...`);
            reject();
            return;
          } else {
            console.log(
              "\u226B This project and its dependencies are not affected"
            );
            console.log("\u226B Ignoring the change");
            resolve(null); // Resolve null to satisfy TypeScript, but we don't use the value.
            return;
          }
        } catch (e) {
          console.error(
            `\u226B Failed to parse JSON output from \`${command}\`.`
          );
          console.error(e);
          console.error(`\u226B Proceeding with build to be safe...`);
          reject();
          return;
        }
      }
    );
  });
}
