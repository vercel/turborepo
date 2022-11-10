import { exec } from "child_process";
import { getTurboRoot } from "turbo-utils";
import { getComparison } from "./getComparison";
import { getWorkspace } from "./getWorkspace";
import { info, error } from "./logger";
import { TurboIgnoreArgs } from "./types";

function ignoreBuild() {
  info(`ignoring the change`);
  return process.exit(0);
}

function continueBuild() {
  info(`proceeding with deployment`);
  return process.exit(1);
}

export default function turboIgnore({ args }: { args: TurboIgnoreArgs }) {
  info(
    "Using Turborepo to determine if this project is affected by the commit...\n"
  );

  // check for TURBO_FORCE and bail early if it's set
  if (process.env.TURBO_FORCE === "true") {
    info("`TURBO_FORCE` detected");
    return continueBuild();
  }

  // find the monorepo root
  const root = getTurboRoot();
  if (!root) {
    error("monorepo root not found. turbo-ignore inferencing failed");
    return continueBuild();
  }

  // Find the workspace from the command-line args, or the package.json at the current directory
  const workspace = getWorkspace({ cwd: process.cwd(), args });
  if (!workspace) {
    error("workspace not found. turbo-ignore inferencing failed");
    return continueBuild();
  }

  // Get the start of the comparison (previous deployment when available, or previous commit by default)
  const comparison = getComparison({ workspace });
  if (!comparison) {
    // This is either the first deploy of the project, or the first deploy for the branch, either way - build it.
    info(
      `no previous deployments found for "${workspace}"${
        process.env.VERCEL === "1"
          ? ` on "${process.env.VERCEL_GIT_COMMIT_REF}".`
          : "."
      }`
    );
    return continueBuild();
  }
  if (comparison.type === "previousDeploy") {
    info("found previous deployment for project");
  }

  // Build, and execute the command
  const command = `npx turbo run build --filter=${workspace}...[${comparison.ref}] --dry=json`;
  info(`analyzing results of \`${command}\``);
  exec(
    command,
    {
      cwd: root,
    },
    (err, stdout) => {
      if (err) {
        error(`exec error: ${err}`);
        return continueBuild();
      }

      try {
        const parsed = JSON.parse(stdout);
        if (parsed == null) {
          error(`failed to parse JSON output from \`${command}\`.`);
          return continueBuild();
        }
        const { packages } = parsed;
        if (packages && packages.length > 0) {
          info(
            `the commit affects this project and/or its ${
              packages.length - 1
            } dependencies`
          );
          return continueBuild();
        } else {
          info(`this project and its dependencies are not affected`);
          return ignoreBuild();
        }
      } catch (e) {
        error(`failed to parse JSON output from \`${command}\`.`);
        error(e);
        return continueBuild();
      }
    }
  );
}
