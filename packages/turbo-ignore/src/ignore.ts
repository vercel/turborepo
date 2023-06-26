import { exec } from "child_process";
import path from "path";
import { getTurboRoot } from "@turbo/utils";
import { getComparison } from "./getComparison";
import { getTask } from "./getTask";
import { getWorkspace } from "./getWorkspace";
import { info, warn, error } from "./logger";
import { shouldWarn } from "./errors";
import { TurboIgnoreArgs } from "./types";
import { checkCommit } from "./checkCommit";

function ignoreBuild() {
  console.log("⏭ Ignoring the change");
  return process.exit(0);
}

function continueBuild() {
  console.log("✓ Proceeding with deployment");
  return process.exit(1);
}

export default function turboIgnore({ args }: { args: TurboIgnoreArgs }) {
  info(
    `Using Turborepo to determine if this project is affected by the commit...\n`
  );

  // set default directory
  args.directory = args.directory
    ? path.resolve(args.directory)
    : process.cwd();

  // check for TURBO_FORCE and bail early if it's set
  if (process.env.TURBO_FORCE === "true") {
    info("`TURBO_FORCE` detected");
    return continueBuild();
  }

  // find the monorepo root
  const root = getTurboRoot(args.directory);
  if (!root) {
    error("Monorepo root not found. turbo-ignore inferencing failed");
    return continueBuild();
  }

  // Find the workspace from the command-line args, or the package.json at the current directory
  const workspace = getWorkspace(args);
  if (!workspace) {
    return continueBuild();
  }

  // Identify which task to execute from the command-line args
  let task = getTask(args);

  // check the commit message
  const parsedCommit = checkCommit({ workspace });
  if (parsedCommit.result === "skip") {
    info(parsedCommit.reason);
    return ignoreBuild();
  }
  if (parsedCommit.result === "deploy") {
    info(parsedCommit.reason);
    return continueBuild();
  }
  if (parsedCommit.result === "conflict") {
    info(parsedCommit.reason);
  }

  // Get the start of the comparison (previous deployment when available, or previous commit by default)
  const comparison = getComparison({ workspace, fallback: args.fallback });
  if (!comparison) {
    // This is either the first deploy of the project, or the first deploy for the branch, either way - build it.
    return continueBuild();
  }

  // Build, and execute the command
  const command = `npx turbo run ${task} --filter=${workspace}...[${comparison.ref}] --dry=json`;
  info(`Analyzing results of \`${command}\``);
  exec(
    command,
    {
      cwd: root,
    },
    (err, stdout) => {
      if (err) {
        const { level, code, message } = shouldWarn({ err: err.message });
        if (level === "warn") {
          warn(message);
        } else {
          error(`${code}: ${err}`);
        }
        return continueBuild();
      }

      try {
        const parsed = JSON.parse(stdout);
        if (parsed == null) {
          error(`Failed to parse JSON output from \`${command}\`.`);
          return continueBuild();
        }
        const { packages } = parsed;
        if (packages && packages.length > 0) {
          if (packages.length === 1) {
            info(`This commit affects "${workspace}"`);
          } else {
            // subtract 1 because the first package is the workspace itself
            info(
              `This commit affects "${workspace}" and ${packages.length - 1} ${
                packages.length - 1 === 1 ? "dependency" : "dependencies"
              } (${packages.slice(1).join(", ")})`
            );
          }

          return continueBuild();
        } else {
          info(`This project and its dependencies are not affected`);
          return ignoreBuild();
        }
      } catch (e) {
        error(`Failed to parse JSON output from \`${command}\`.`);
        error(e);
        return continueBuild();
      }
    }
  );
}
