#!/usr/bin/env node

import {
  checkProjectReferences,
  getProjectReferenceCandidates,
  initProjectReferences,
  writeProjectReferences
} from "./index";
import { ProjectReferencesError, type ProjectReferencesResult } from "./types";

interface CliFlags {
  json: boolean;
  verbose: boolean;
  dryRun: boolean;
  force: boolean;
  cwd?: string;
}

const COMMANDS = new Set(["init", "check", "write", "candidates"]);

async function main(argv: Array<string>) {
  const { command, flags } = parseArgs(argv);
  let result: ProjectReferencesResult;

  switch (command) {
    case "init": {
      result = await initProjectReferences(flags);
      break;
    }
    case "check": {
      result = await checkProjectReferences(flags);
      break;
    }
    case "write": {
      result = await writeProjectReferences(flags);
      break;
    }
    case "candidates": {
      result = await getProjectReferenceCandidates(flags);
      break;
    }
  }

  printResult(result, flags);
  process.exitCode = result.success ? 0 : 1;
}

function parseArgs(argv: Array<string>): {
  command: "init" | "check" | "write" | "candidates";
  flags: CliFlags;
} {
  if (argv[0] !== "project-references" || !COMMANDS.has(argv[1] ?? "")) {
    printHelp();
    process.exit(1);
  }

  const flags: CliFlags = {
    json: false,
    verbose: false,
    dryRun: false,
    force: false
  };
  for (let index = 2; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--json") {
      flags.json = true;
    } else if (arg === "--verbose") {
      flags.verbose = true;
    } else if (arg === "--dry-run") {
      flags.dryRun = true;
    } else if (arg === "--force") {
      flags.force = true;
    } else if (arg === "--cwd") {
      const cwd = argv[index + 1];
      if (!cwd) {
        throw new ProjectReferencesError("--cwd requires a path");
      }
      flags.cwd = cwd;
      index += 1;
    } else if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    } else {
      throw new ProjectReferencesError(`Unknown argument ${arg}`);
    }
  }

  return {
    command: argv[1] as "init" | "check" | "write" | "candidates",
    flags
  };
}

function printResult(result: ProjectReferencesResult, flags: CliFlags) {
  if (flags.json) {
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    return;
  }

  const changed = result.dryRun ? "would update" : "updated";
  if (result.changedFiles.length > 0) {
    process.stdout.write(
      `${result.command}: ${changed} ${result.changedFiles.length} file(s).\n`
    );
    process.stdout.write(result.dryRun ? "Files to update:\n" : "Updated files:\n");
    for (const file of result.changedFiles) {
      process.stdout.write(`  ${file}\n`);
    }
  } else {
    process.stdout.write(
      `${result.command}: Project References are up to date.\n`
    );
  }

  process.stdout.write(
    `Packages: ${result.summary.validCount} valid, ${result.summary.excludedCount} excluded, ${result.summary.ignoredCount} ignored.\n`
  );

  if (result.candidates.length > 0) {
    process.stdout.write("Candidates:\n");
    for (const candidate of result.candidates) {
      process.stdout.write(`  ${candidate}\n`);
    }
  }
  if (result.newPackages.length > 0) {
    process.stdout.write("New packages:\n");
    for (const pkg of result.newPackages) {
      process.stdout.write(`  ${pkg}\n`);
    }
  }

  for (const diagnostic of result.diagnostics) {
    process.stdout.write(`${diagnostic.level}: ${diagnostic.message}\n`);
    if (flags.verbose && diagnostic.details) {
      for (const detail of diagnostic.details) {
        process.stdout.write(`  ${detail}\n`);
      }
    }
  }
}

function printHelp() {
  process.stdout.write(
    `Usage: turbo-typescript project-references <command> [options]\n\n`
  );
  process.stdout.write(`Commands: init, check, write, candidates\n`);
  process.stdout.write(
    `Options: --json --verbose --dry-run --force --cwd <path>\n`
  );
}

main(process.argv.slice(2)).catch((error) => {
  const diagnostics =
    error instanceof ProjectReferencesError
      ? error.diagnostics
      : [
          {
            level: "error" as const,
            code: "unexpected_error",
            message: error instanceof Error ? error.message : String(error)
          }
        ];

  if (process.argv.includes("--json")) {
    process.stdout.write(
      `${JSON.stringify(
        {
          version: 1,
          success: false,
          diagnostics
        },
        null,
        2
      )}\n`
    );
  } else {
    for (const diagnostic of diagnostics) {
      process.stderr.write(`${diagnostic.level}: ${diagnostic.message}\n`);
    }
  }
  process.exitCode = 1;
});
