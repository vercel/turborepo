import { exec } from "node:child_process";
import path from "node:path";
import { existsSync } from "node:fs";
import { getTurboRoot } from "@turbo/utils";
import type { DryRun } from "@turbo/types";
import { getComparison } from "./getComparison";
import { getTask } from "./getTask";
import { getWorkspace } from "./getWorkspace";
import { log, info, warn, error } from "./logger";
import { shouldWarn } from "./errors";
import type { TurboIgnoreArg, TurboIgnoreOptions } from "./types";
import { checkCommit } from "./checkCommit";

function ignoreBuild() {
  log("⏭ Ignoring the change");
  return process.exit(0);
}

function continueBuild() {
  log("✓ Proceeding with deployment");
  return process.exit(1);
}

export function turboIgnore(
  workspaceArg: TurboIgnoreArg,
  opts: TurboIgnoreOptions
) {
  const inputs = {
    workspace: workspaceArg,
    ...opts,
  };

  info(
    `Using Turborepo to determine if this project is affected by the commit...\n`
  );

  // set default directory
  if (opts.directory) {
    const directory = path.resolve(opts.directory);
    if (existsSync(directory)) {
      inputs.directory = directory;
    } else {
      warn(
        `Directory "${opts.directory}" does not exist, using current directory`
      );
      inputs.directory = process.cwd();
    }
  } else {
    inputs.directory = process.cwd();
  }

  // check for TURBO_FORCE and bail early if it's set
  if (process.env.TURBO_FORCE === "true") {
    info("`TURBO_FORCE` detected");
    return continueBuild();
  }

  // find the monorepo root
  const root = getTurboRoot(inputs.directory);
  if (!root) {
    error("Monorepo root not found. turbo-ignore inferencing failed");
    return continueBuild();
  }

  // Find the workspace from the command-line args, or the package.json at the current directory
  const workspace = getWorkspace(inputs);
  if (!workspace) {
    return continueBuild();
  }

  // Identify which task to execute from the command-line args
  const task = getTask(inputs);

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
  const comparison = getComparison({ workspace, fallback: inputs.fallback });
  if (!comparison) {
    // This is either the first deploy of the project, or the first deploy for the branch, either way - build it.
    return continueBuild();
  }

  // Build, and execute the command
  const command = `npx turbo run ${task} --filter="${workspace}...[${comparison.ref}]" --dry=json`;
  info(`Analyzing results of \`${command}\``);

  const execOptions: { cwd: string; maxBuffer?: number } = {
    cwd: root,
  };

  if (opts.maxBuffer) {
    execOptions.maxBuffer = opts.maxBuffer;
  }

  exec(command, execOptions, (err, stdout) => {
    if (err) {
      const { level, code, message } = shouldWarn({ err: err.message });
      if (level === "warn") {
        warn(message);
      } else {
        error(`${code}: ${err.message}`);
      }
      return continueBuild();
    }

    try {
      const parsed = JSON.parse(stdout) as DryRun | null;
      if (parsed === null) {
        error(`Failed to parse JSON output from \`${command}\`.`);
        return continueBuild();
      }
      const { packages } = parsed;
      if (packages.length > 0) {
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
      }
      info(`This project and its dependencies are not affected`);
      return ignoreBuild();
    } catch (e) {
      error(`Failed to parse JSON output from \`${command}\`.`);
      error(e);
      return continueBuild();
    }
  });
}
