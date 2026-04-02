import os from "node:os";
import { execSync } from "node:child_process";
import picocolors from "picocolors";
import { input } from "@inquirer/prompts";
import { getWorkspaceDetails, type Project } from "@turbo/workspaces";
import { logger } from "@turbo/utils";
import { checkGitStatus } from "../../utils/check-git-status";
import { directoryInfo } from "../../utils/directory-info";
import { Runner } from "../../runner/runner";
import { looksLikeRepo } from "../../utils/looks-like-repo";
import type { TransformerResults } from "../../runner";
import { transformer as updateVersionedSchema } from "../../transforms/update-versioned-schema-json";
import { getCurrentVersion } from "./steps/get-current-version";
import { getLatestVersion } from "./steps/get-latest-version";
import { getTransformsForMigration } from "./steps/get-transforms-for-migration";
import { getTurboUpgradeCommand } from "./steps/get-turbo-upgrade-command";
import {
  detectCatalog,
  updateCatalogVersion,
  type CatalogInfo
} from "./steps/update-catalog";
import type { MigrateCommandArgument, MigrateCommandOptions } from "./types";
import { shutdownDaemon } from "./steps/shutdown-daemon";

function endMigration({
  message,
  success
}: {
  message?: string;
  success: boolean;
}) {
  if (success) {
    logger.bold(picocolors.green("Migration completed"));
    if (message) {
      logger.log(message);
    }
    return process.exit(0);
  }

  logger.bold(picocolors.red("Migration failed"));
  if (message) {
    logger.log(message);
  }
  return process.exit(1);
}

/**
Migration is done in 4 steps:
  -- gather information
  1. find the version (x) of turbo to migrate from (if not specified)
  2. find the version (y) of turbo to migrate to (if not specified)
  3. determine which codemods need to be run to move from version x to version y
  -- action
  4. execute the codemods (serially, and in order)
  5. update the turbo version (optionally)
**/
export async function migrate(
  directory: MigrateCommandArgument,
  options: MigrateCommandOptions
) {
  // check git status
  if (!options.dryRun) {
    checkGitStatus({ directory, force: options.force });
  }

  let selectedDirectory = directory;
  if (!selectedDirectory) {
    selectedDirectory = await input({
      message: "Where is the root of the repo to migrate?",
      default: ".",
      validate: (d: string) => {
        const { exists, absolute } = directoryInfo({ directory: d });
        if (exists) {
          return true;
        }
        return `Directory ${picocolors.dim(`(${absolute})`)} does not exist`;
      },
      transformer: (d: string) => d.trim()
    });
    selectedDirectory = selectedDirectory.trim();
  }
  const { exists, absolute: root } = directoryInfo({
    directory: selectedDirectory
  });
  if (!exists) {
    return endMigration({
      success: false,
      message: `Directory ${picocolors.dim(`(${root})`)} does not exist`
    });
  }

  if (!looksLikeRepo({ directory: root })) {
    return endMigration({
      success: false,
      message: `Directory (${picocolors.dim(
        root
      )}) does not appear to be a repository`
    });
  }

  let project: Project | undefined;
  try {
    project = await getWorkspaceDetails({ root });
  } catch (err) {
    return endMigration({
      success: false,
      message: `Unable to read determine package manager details from ${picocolors.dim(
        root
      )}`
    });
  }

  // step 1
  const fromVersion = getCurrentVersion(project, options);
  if (!fromVersion) {
    return endMigration({
      success: false,
      message: `Unable to infer the version of turbo being used by ${project.name}`
    });
  }

  // step 2
  let toVersion = options.to;
  try {
    toVersion = await getLatestVersion(options);
  } catch (err) {
    let message = "UNKNOWN_ERROR";
    if (err instanceof Error) {
      message = err.message;
    }
    return endMigration({
      success: false,
      message
    });
  }

  if (!toVersion) {
    return endMigration({
      success: false,
      message: "Unable to fetch the latest version of turbo"
    });
  }

  if (fromVersion === toVersion) {
    return endMigration({
      success: true,
      message: `Nothing to do, current version (${picocolors.bold(
        fromVersion
      )}) is the same as the requested version (${picocolors.bold(toVersion)})`
    });
  }

  // step 3
  const codemods = getTransformsForMigration({ fromVersion, toVersion });
  if (codemods.length === 0) {
    logger.log(
      `No codemods required to migrate from ${fromVersion} to ${toVersion}`,
      os.EOL
    );
  }

  // shutdown the turbo daemon before running codemods and upgrading
  // the daemon can handle version mismatches, but we do this as an extra precaution
  if (!options.dryRun) {
    shutdownDaemon({ project });
  }

  // step 4
  logger.log(
    `Upgrading turbo from ${picocolors.bold(fromVersion)} to ${picocolors.bold(
      toVersion
    )} (${
      codemods.length === 0
        ? "no codemods required"
        : `${codemods.length} required codemod${
            codemods.length === 1 ? "" : "s"
          }`
    })`,
    os.EOL
  );

  const results: Array<TransformerResults> = [];
  for (const [idx, codemod] of codemods.entries()) {
    logger.log(
      `(${idx + 1}/${codemods.length}) ${picocolors.bold(
        `Running ${codemod.name}`
      )}`
    );

    // eslint-disable-next-line no-await-in-loop -- transforms have to run serially to avoid conflicts
    const result = await codemod.transformer({
      root: project.paths.root,
      options: { ...options, toVersion }
    });
    Runner.logResults(result);
    results.push(result);
  }

  // Always update the $schema URL to match the target version.
  // The versioned schema transform may not be selected by getTransformsForMigration
  // during same-major migrations (e.g., 2.8.0 -> 2.9.3) since its introducedIn
  // version has already passed. Running it here ensures the schema URL stays in sync.
  const VERSIONED_SCHEMA_TRANSFORM = "update-versioned-schema-json";
  if (!codemods.some((c) => c.name === VERSIONED_SCHEMA_TRANSFORM)) {
    const schemaResult = updateVersionedSchema({
      root: project.paths.root,
      options: { ...options, toVersion }
    });
    Runner.logResults(schemaResult);
    results.push(schemaResult);
  }

  const hasTransformError = results.some(
    (result) =>
      result.fatalError ||
      Object.keys(result.changes).some((key) => result.changes[key].error)
  );

  if (hasTransformError) {
    return endMigration({
      success: false,
      message:
        "Could not complete migration due to codemod errors. Please fix the errors and try again."
    });
  }

  // step 5

  // Check if turbo uses a catalog reference (e.g. "turbo": "catalog:" in package.json).
  // If so, update the version in the catalog file instead of letting the package
  // manager overwrite the catalog reference with a literal version.
  let catalogInfo: CatalogInfo | undefined;
  if (project) {
    catalogInfo = detectCatalog({
      root: project.paths.root,
      packageManager: project.packageManager
    });
  }

  if (catalogInfo) {
    if (options.dryRun) {
      logger.log(
        `Would update turbo version in catalog file ${picocolors.dim(
          catalogInfo.catalogFile
        )} ${picocolors.dim("(dry run)")}`,
        os.EOL
      );
    } else {
      const updated = updateCatalogVersion({
        catalogInfo,
        version: toVersion
      });
      if (updated) {
        logger.log(
          `Updated turbo version to ${picocolors.bold(
            toVersion
          )} in ${picocolors.dim(catalogInfo.catalogFile)}`,
          os.EOL
        );
      }
    }
  }

  // find the upgrade command, and run it
  const upgradeCommand = await getTurboUpgradeCommand({
    project,
    to: options.to,
    catalogInfo
  });

  if (!upgradeCommand) {
    return endMigration({
      success: false,
      message: "Unable to determine upgrade command"
    });
  }

  // install
  if (options.install) {
    if (options.dryRun) {
      logger.log(
        `Upgrading turbo with ${picocolors.bold(
          upgradeCommand
        )} ${picocolors.dim("(dry run)")}`,
        os.EOL
      );
    } else {
      logger.log(
        `Upgrading turbo with ${picocolors.bold(upgradeCommand)}`,
        os.EOL
      );
      try {
        execSync(upgradeCommand, { stdio: "pipe", cwd: project.paths.root });
      } catch (err: unknown) {
        return endMigration({
          success: false,
          message: `Unable to upgrade turbo: ${String(err)}`
        });
      }
    }
  } else {
    logger.log(`Upgrade turbo with ${picocolors.bold(upgradeCommand)}`, os.EOL);
  }

  endMigration({ success: true });
}
