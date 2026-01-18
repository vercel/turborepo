/**
 * @module update-versioned-schema-json
 *
 * Migrates turbo.json $schema URLs from legacy formats to versioned subdomains.
 *
 * ## Migration Path
 * Legacy URLs:
 * - `https://turborepo.dev/schema.json` -> `https://v{X}-{Y}-{Z}.turborepo.dev/schema.json`
 * - `https://turborepo.dev/schema.v2.json` -> `https://v{X}-{Y}-{Z}.turborepo.dev/schema.json`
 * - `https://turborepo.com/schema.json` -> `https://v{X}-{Y}-{Z}.turborepo.dev/schema.json`
 * - `https://turborepo.com/schema.v2.json` -> `https://v{X}-{Y}-{Z}.turborepo.dev/schema.json`
 * - `https://turbo.build/schema.json` -> `https://v{X}-{Y}-{Z}.turborepo.dev/schema.json`
 * - `https://turbo.build/schema.v2.json` -> `https://v{X}-{Y}-{Z}.turborepo.dev/schema.json`
 *
 * Outdated versioned URLs:
 * - `https://v{A}-{B}-{C}.turborepo.dev/schema.json` -> `https://v{X}-{Y}-{Z}.turborepo.dev/schema.json`
 *
 * ## Relationship to update-schema-json-url
 * - `update-schema-json-url` (introduced 2.0.0): Handles schema.v1.json -> schema.v2.json
 * - This transformer (introduced 2.7.5): Handles legacy URLs -> versioned subdomains
 *
 * Both run during migrations. For a 1.x -> 2.8.x migration:
 * 1. update-schema-json-url runs first (v1 -> v2)
 * 2. This transformer runs second (schema.v2.json -> v2-8-0.turborepo.dev)
 *
 * ## Version Gating
 * Two constants control when this transformer runs:
 * - INTRODUCED_IN: Controls when this codemod is included in migrations (used by getTransformsForMigration)
 * - MIN_VERSIONED_SCHEMA_VERSION: Runtime check to skip if target version doesn't support versioned URLs
 *
 * These are currently identical but could diverge if this transformer is backported.
 *
 * ## Idempotency
 * Marked idempotent because running twice with the same toVersion is a no-op.
 */

import path from "node:path";
import fs from "fs-extra";
import { gte, coerce } from "semver";
import { getTurboConfigs } from "@turbo/utils";
import type { TransformerResults } from "../runner";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";
import type { Transformer, TransformerArgs } from "../types";

// transformer details
const TRANSFORMER = "update-versioned-schema-json";
const DESCRIPTION =
  'Update the "$schema" property in turbo.json to use the versioned subdomain format (e.g., https://v2-7-5.turborepo.dev/schema.json)';

// INTRODUCED_IN: Controls when this codemod is included in migrations (via getTransformsForMigration)
const INTRODUCED_IN = "2.7.5";

// MIN_VERSIONED_SCHEMA_VERSION: Runtime check - skip if target version doesn't support versioned URLs.
// Currently identical to INTRODUCED_IN but could diverge if this transformer is backported.
const MIN_VERSIONED_SCHEMA_VERSION = INTRODUCED_IN;

// Old schema URL patterns to migrate (static strings).
// NOTE: Intentionally excludes schema.v1.json - that's handled by update-schema-json-url
const OLD_SCHEMA_URLS = [
  "https://turborepo.dev/schema.json",
  "https://turborepo.dev/schema.v2.json",
  "https://turborepo.com/schema.json",
  "https://turborepo.com/schema.v2.json",
  "https://turbo.build/schema.json",
  "https://turbo.build/schema.v2.json"
];

// Regex to match existing versioned schema URLs (e.g., https://v2-7-4.turborepo.dev/schema.json)
const VERSIONED_SCHEMA_URL_REGEX =
  /https:\/\/v\d+-\d+-\d+\.turborepo\.dev\/schema\.json/g;

/**
 * Extracts the base version (major.minor.patch) from a semver string,
 * stripping any prerelease or build metadata.
 * e.g., "2.7.5-canary.13" -> "2.7.5"
 */
function getBaseVersion(version: string): string | null {
  const coerced = coerce(version);
  return coerced ? coerced.version : null;
}

/**
 * Converts a semver version to the subdomain format (e.g., "2.7.5" -> "v2-7-5")
 */
function versionToSubdomain(version: string): string {
  const [major, minor, patch] = version.split(".");
  return `v${major}-${minor}-${patch}`;
}

/**
 * Generates the new versioned schema URL
 */
function getVersionedSchemaUrl(version: string): string {
  const baseVersion = getBaseVersion(version);
  if (!baseVersion) {
    throw new Error(`Invalid version: ${version}`);
  }
  const subdomain = versionToSubdomain(baseVersion);
  return `https://${subdomain}.turborepo.dev/schema.json`;
}

/**
 * Updates any old schema URLs in file content to the new versioned URL
 */
function updateSchemaUrls(content: string, newUrl: string): string {
  let updated = content;
  // Replace static old URLs
  for (const oldUrl of OLD_SCHEMA_URLS) {
    updated = updated.replaceAll(oldUrl, newUrl);
  }
  // Replace outdated versioned URLs (e.g., v2-7-4 -> v2-7-5)
  updated = updated.replaceAll(VERSIONED_SCHEMA_URL_REGEX, newUrl);
  return updated;
}

/**
 * Checks if the content contains any schema URLs that need updating
 */
function hasUpdatableSchemaUrl(content: string, newUrl: string): boolean {
  // Check for static old URLs
  if (OLD_SCHEMA_URLS.some((url) => content.includes(url))) {
    return true;
  }
  // Check for outdated versioned URLs (any versioned URL that isn't the target)
  const matches = content.match(VERSIONED_SCHEMA_URL_REGEX);
  if (matches) {
    return matches.some((match) => match !== newUrl);
  }
  return false;
}

export function transformer({
  root,
  options
}: TransformerArgs): TransformerResults {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options
  });

  const { toVersion } = options;

  // Get base version (strips prerelease/build metadata)
  const baseVersion = toVersion ? getBaseVersion(toVersion) : null;

  // If no version specified or version is below minimum, skip
  if (!baseVersion || !gte(baseVersion, MIN_VERSIONED_SCHEMA_VERSION)) {
    log.info(
      `Skipping schema URL update: target version ${toVersion || "unknown"} does not support versioned schema URLs`
    );
    return runner.finish();
  }

  log.info(
    'Updating "$schema" property in turbo.json files to versioned format...'
  );

  const rootTurboConfigPath = path.join(root, "turbo.json");
  if (!fs.existsSync(rootTurboConfigPath)) {
    return runner.abortTransform({
      reason: `No turbo.json found at ${root}. Is the path correct?`
    });
  }

  try {
    const newSchemaUrl = getVersionedSchemaUrl(baseVersion);

    // Get all turbo.json files (root + workspaces)
    const allTurboJsons = getTurboConfigs(root);

    for (const { turboConfigPath } of allTurboJsons) {
      // Read turbo.json as string to preserve formatting
      const turboConfigContent = fs.readFileSync(turboConfigPath, "utf8");

      // Check if it has any schema URL that needs updating
      if (hasUpdatableSchemaUrl(turboConfigContent, newSchemaUrl)) {
        const updatedContent = updateSchemaUrls(
          turboConfigContent,
          newSchemaUrl
        );

        runner.modifyFile({
          filePath: turboConfigPath,
          before: turboConfigContent,
          after: updatedContent
        });

        log.info(`Updated "$schema" in ${turboConfigPath}`);
      }
    }
  } catch (err) {
    return runner.abortTransform({
      reason: `Error updating schema URL: ${String(err)}`
    });
  }

  return runner.finish();
}

const transformerMeta: Transformer = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer,
  idempotent: true
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
