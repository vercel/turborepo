import path from "node:path";
import fs from "fs-extra";
import { parseDocument } from "yaml";
import { logger, type PackageManager, type PackageJson } from "@turbo/utils";

type InstallType = "dependencies" | "devDependencies";

export interface CatalogInfo {
  /** Which catalog the reference points to (null = default) */
  catalogName: string | null;
  /** Absolute path to the catalog file */
  catalogFile: string;
  /** Which dependency group turbo is in */
  installType: InstallType;
}

const CATALOG_FILES: Partial<Record<PackageManager, string>> = {
  pnpm: "pnpm-workspace.yaml",
  yarn: ".yarnrc.yml"
};

/**
 * Detect if turbo is installed via a `catalog:` protocol reference.
 * Returns catalog metadata if so, undefined otherwise.
 */
export function detectCatalog({
  root,
  packageManager
}: {
  root: string;
  packageManager: PackageManager;
}): CatalogInfo | undefined {
  const packageJsonPath = path.join(root, "package.json");
  if (!fs.existsSync(packageJsonPath)) {
    return undefined;
  }

  const packageJson = fs.readJsonSync(packageJsonPath) as PackageJson;

  let turboSpecifier: string | undefined;
  let installType: InstallType;

  if (packageJson.devDependencies && "turbo" in packageJson.devDependencies) {
    turboSpecifier = packageJson.devDependencies.turbo;
    installType = "devDependencies";
  } else if (packageJson.dependencies && "turbo" in packageJson.dependencies) {
    turboSpecifier = packageJson.dependencies.turbo;
    installType = "dependencies";
  } else {
    return undefined;
  }

  if (!turboSpecifier?.startsWith("catalog:")) {
    return undefined;
  }

  const catalogFileName = CATALOG_FILES[packageManager];
  if (!catalogFileName) {
    return undefined;
  }

  const catalogFile = path.join(root, catalogFileName);
  if (!fs.existsSync(catalogFile)) {
    logger.warn(
      `Found catalog reference but ${catalogFileName} does not exist`
    );
    return undefined;
  }

  const ref = turboSpecifier.slice("catalog:".length);
  const catalogName = ref === "" || ref === "default" ? null : ref;

  return { catalogName, catalogFile, installType };
}

/**
 * Update the turbo version in a catalog file (pnpm-workspace.yaml or .yarnrc.yml).
 * Preserves the existing version range prefix (^, ~, etc.) and YAML formatting.
 *
 * Returns true if the catalog was updated.
 */
export function updateCatalogVersion({
  catalogInfo,
  version
}: {
  catalogInfo: CatalogInfo;
  version: string;
}): boolean {
  const { catalogFile, catalogName } = catalogInfo;
  const content = fs.readFileSync(catalogFile, "utf8");
  const doc = parseDocument(content);

  // Path differs for default vs named catalogs:
  //   default: catalog.turbo
  //   named:   catalogs.<name>.turbo
  const yamlPath =
    catalogName === null
      ? ["catalog", "turbo"]
      : ["catalogs", catalogName, "turbo"];

  const currentValue = doc.getIn(yamlPath) as string | undefined;
  if (!currentValue) {
    logger.warn(
      `Could not find turbo in ${catalogName ? `catalog "${catalogName}"` : "default catalog"} in ${catalogFile}`
    );
    return false;
  }

  // Preserve the version range prefix (^, ~, >=, etc.)
  const prefixMatch = currentValue.match(/^([^\d]*)/);
  const prefix = prefixMatch ? prefixMatch[1] : "^";
  const newValue = `${prefix}${version}`;

  if (currentValue === newValue) {
    return false;
  }

  doc.setIn(yamlPath, newValue);
  fs.writeFileSync(catalogFile, doc.toString());

  return true;
}
