import path from "node:path";

import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  findPackageJsonFiles,
  getExamplePath,
  getRepoRoot,
  isJsonObject,
  packageManagerName,
  readJsonFile
} from "../lib/repo.js";

const dependencyFields = [
  "dependencies",
  "devDependencies",
  "peerDependencies",
  "optionalDependencies"
] as const;

interface PackageVersionFinding {
  path: string;
  field: string;
  name: string;
  current: string;
  latest: string | null;
  recommended: string | null;
  required: string | null;
  needsUpdate: boolean;
}

interface PackageManagerFinding {
  path: string;
  current: string | null;
  latest: string | null;
  recommended: string | null;
  required: string | null;
  needsUpdate: boolean;
}

interface NodeFinding {
  path: string;
  current: string | null;
  latestLts: string | null;
}

const packageMetadataCache = new Map<string, Promise<string | null>>();

export default defineTool({
  description:
    "Audit one example, or all examples, for stale dependencies, packageManager pins, and Node engine ranges using npm and Node release metadata.",
  inputSchema: z.object({
    example: z
      .string()
      .min(1)
      .optional()
      .describe(
        "Optional directory name under examples/. Omit to audit every example."
      ),
    includeDependencies: z
      .boolean()
      .default(true)
      .describe(
        "Whether to check dependency fields against npm latest dist-tags."
      ),
    includePackageManager: z
      .boolean()
      .default(true)
      .describe(
        "Whether to check packageManager pins against latest stable package-manager releases."
      ),
    includeNode: z
      .boolean()
      .default(true)
      .describe(
        "Whether to include the latest active Node LTS version for engines.node review."
      )
  }),
  async execute({
    example,
    includeDependencies,
    includePackageManager,
    includeNode
  }) {
    const repoRoot = await getRepoRoot();
    const auditRoot = example
      ? await getExamplePath(example)
      : path.join(repoRoot, "examples");
    const packageJsonFiles = await findPackageJsonFiles(auditRoot);
    const latestNodeLts = includeNode ? await fetchLatestNodeLts() : null;
    const dependencies: PackageVersionFinding[] = [];
    const packageManagers: PackageManagerFinding[] = [];
    const nodeEngines: NodeFinding[] = [];

    for (const packageJsonFile of packageJsonFiles) {
      const packageJson = await readJsonFile(packageJsonFile);
      const relativePath = path.relative(repoRoot, packageJsonFile);

      if (includePackageManager) {
        const packageManager =
          typeof packageJson.packageManager === "string"
            ? packageJson.packageManager
            : null;
        const latestPackageManager = packageManager
          ? await fetchLatestPackageManagerVersion(packageManager)
          : null;
        const recommendedPackageManager = recommendedPackageManagerPin(
          packageManager,
          latestPackageManager
        );
        packageManagers.push({
          path: relativePath,
          current: packageManager,
          latest: latestPackageManager,
          recommended: recommendedPackageManager,
          required: recommendedPackageManager,
          needsUpdate:
            packageManager !== null &&
            recommendedPackageManager !== null &&
            packageManager !== recommendedPackageManager
        });
      }

      if (includeNode) {
        const engines = isJsonObject(packageJson.engines)
          ? packageJson.engines
          : null;
        nodeEngines.push({
          path: relativePath,
          current: typeof engines?.node === "string" ? engines.node : null,
          latestLts: latestNodeLts
        });
      }

      if (!includeDependencies) {
        continue;
      }

      for (const field of dependencyFields) {
        const fieldValue = packageJson[field];
        if (!isJsonObject(fieldValue)) {
          continue;
        }

        for (const [name, current] of Object.entries(fieldValue)) {
          if (typeof current !== "string" || shouldSkipVersion(current)) {
            continue;
          }

          const latest = await fetchLatestNpmVersion(name);
          dependencies.push({
            path: relativePath,
            field,
            name,
            current,
            latest,
            recommended: latest,
            required: latest,
            needsUpdate: latest !== null && current !== latest
          });
        }
      }
    }

    return {
      auditedPackageJsonFiles: packageJsonFiles.map((file) =>
        path.relative(repoRoot, file)
      ),
      packageManagers,
      nodeEngines,
      dependencies
    };
  }
});

async function fetchLatestPackageManagerVersion(
  packageManager: string
): Promise<string | null> {
  const manager = packageManagerName(packageManager);
  if (manager === "yarn" && packageManager.startsWith("yarn@1.")) {
    return fetchLatestNpmVersion("yarn");
  }
  if (manager === "yarn") {
    return fetchLatestNpmVersion("@yarnpkg/cli");
  }
  if (manager === "pnpm" || manager === "npm") {
    return fetchLatestNpmVersion(manager);
  }
  return null;
}

function recommendedPackageManagerPin(
  packageManager: string | null,
  latest: string | null
): string | null {
  const manager = packageManagerName(packageManager);
  if (manager === null || latest === null) {
    return null;
  }
  return `${manager}@${latest}`;
}

async function fetchLatestNpmVersion(
  packageName: string
): Promise<string | null> {
  const cached = packageMetadataCache.get(packageName);
  if (cached) {
    return cached;
  }

  const promise = fetch(
    `https://registry.npmjs.org/${encodeURIComponent(packageName)}`
  )
    .then(async (response) => {
      if (!response.ok) {
        return null;
      }
      const metadata: unknown = await response.json();
      if (!isJsonObject(metadata) || !isJsonObject(metadata["dist-tags"])) {
        return null;
      }
      const latest = metadata["dist-tags"].latest;
      return typeof latest === "string" ? latest : null;
    })
    .catch(() => null);

  packageMetadataCache.set(packageName, promise);
  return promise;
}

async function fetchLatestNodeLts(): Promise<string | null> {
  try {
    const response = await fetch("https://nodejs.org/dist/index.json");
    if (!response.ok) {
      return null;
    }
    const releases: unknown = await response.json();
    if (!Array.isArray(releases)) {
      return null;
    }

    for (const release of releases) {
      if (
        !isJsonObject(release) ||
        release.lts === false ||
        typeof release.version !== "string"
      ) {
        continue;
      }
      return release.version.replace(/^v/, "");
    }
  } catch {
    return null;
  }

  return null;
}

function shouldSkipVersion(version: string): boolean {
  return (
    version.startsWith("workspace:") ||
    version.startsWith("file:") ||
    version.startsWith("link:") ||
    version.startsWith("portal:") ||
    version === "*"
  );
}
