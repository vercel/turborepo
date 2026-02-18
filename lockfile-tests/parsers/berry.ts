import type { WorkspaceInfo, FixtureInfo } from "./types";

/**
 * Berry/Yarn 4 lockfiles are YAML-ish but not strict YAML. They have:
 * - __metadata section at the top
 * - Entries keyed by descriptor strings (possibly comma-separated)
 * - Workspace entries have `linkType: soft` and `languageName: unknown`
 * - Resolution strings like `name@workspace:path` identify workspaces
 *
 * We do a line-by-line parse rather than using a YAML parser because
 * some entry keys contain characters that confuse standard YAML parsers
 * (especially comma-separated keys).
 */

interface BerryEntry {
  descriptorLine: string;
  resolution?: string;
  version?: string;
  linkType?: string;
  languageName?: string;
  dependencies: Record<string, string>;
  peerDependencies: Record<string, string>;
}

function parseBerryEntries(content: string): BerryEntry[] {
  const lines = content.split("\n");
  const entries: BerryEntry[] = [];
  let current: BerryEntry | null = null;
  let currentSection: "dependencies" | "peerDependencies" | null = null;

  for (const line of lines) {
    // Skip comments and blank lines
    if (line.startsWith("#") || line.trim() === "") {
      continue;
    }

    // __metadata section - skip it
    if (line.startsWith("__metadata:")) {
      current = null;
      currentSection = null;
      continue;
    }

    // New top-level entry (starts with a quote or non-space character, ends with colon)
    if (/^"[^"]+":?\s*$/.test(line) || /^[a-zA-Z@][^\s]*:\s*$/.test(line)) {
      if (current) entries.push(current);
      current = {
        descriptorLine: line.replace(/:?\s*$/, "").replace(/^"|"$/g, ""),
        dependencies: {},
        peerDependencies: {}
      };
      currentSection = null;
      continue;
    }

    if (!current) continue;

    const trimmed = line.trimStart();
    const indent = line.length - trimmed.length;

    // Properties at indent level 2 (directly under the entry)
    if (indent === 2 && !trimmed.startsWith("#")) {
      currentSection = null;

      if (trimmed.startsWith("resolution:")) {
        const match = trimmed.match(/resolution:\s*"?([^"]+)"?/);
        if (match) current.resolution = match[1];
      } else if (trimmed.startsWith("version:")) {
        const match = trimmed.match(/version:\s*(.+)/);
        if (match) current.version = match[1].trim();
      } else if (trimmed.startsWith("linkType:")) {
        const match = trimmed.match(/linkType:\s*(\S+)/);
        if (match) current.linkType = match[1];
      } else if (trimmed.startsWith("languageName:")) {
        const match = trimmed.match(/languageName:\s*(\S+)/);
        if (match) current.languageName = match[1];
      } else if (trimmed === "dependencies:") {
        currentSection = "dependencies";
      } else if (trimmed === "peerDependencies:") {
        currentSection = "peerDependencies";
      }
      continue;
    }

    // Dependency entries at indent level 4
    if (indent === 4 && currentSection) {
      const depMatch = trimmed.match(
        /^(?:"([^"]+)"|([^:]+)):\s*(?:"([^"]+)"|(.+))$/
      );
      if (depMatch) {
        const depName = depMatch[1] || depMatch[2];
        const depVersion = (depMatch[3] || depMatch[4]).trim();
        current[currentSection][depName] = depVersion;
      }
    }
  }

  if (current) entries.push(current);
  return entries;
}

function parseWorkspaceResolution(resolution: string): {
  name: string;
  path: string;
} | null {
  // Patterns: "name@workspace:path" or "name@workspace:."
  const match = resolution.match(/^(.+?)@workspace:(.+)$/);
  if (!match) return null;
  return { name: match[1], path: match[2] };
}

function extractMetadataVersion(content: string): string {
  const match = content.match(/__metadata:\s*\n\s*version:\s*(\d+)/);
  return match ? match[1] : "6";
}

function detectPatches(content: string): {
  hasPatches: boolean;
  patchFiles: string[];
} {
  // Berry patches appear as resolutions like "name@patch:..."
  const patchFiles: string[] = [];
  const patchMatches = content.matchAll(
    /resolution:\s*"[^"]*@patch:[^"]*#[./~]([^#"]+)"/g
  );
  for (const match of patchMatches) {
    if (match[1]) patchFiles.push(match[1]);
  }
  return {
    hasPatches: patchFiles.length > 0,
    patchFiles
  };
}

export function parseBerryLockfile(
  content: string,
  filename: string,
  filepath: string
): FixtureInfo {
  const entries = parseBerryEntries(content);
  const workspaces: WorkspaceInfo[] = [];
  const metadataVersion = extractMetadataVersion(content);

  for (const entry of entries) {
    if (entry.linkType !== "soft") continue;

    const resolution = entry.resolution;
    if (!resolution) continue;

    const parsed = parseWorkspaceResolution(resolution);
    if (!parsed) continue;

    // Berry doesn't distinguish deps from devDeps in the lockfile.
    // We put everything in dependencies since for frozen install purposes
    // the distinction doesn't matter.
    workspaces.push({
      path: parsed.path,
      name: parsed.name,
      dependencies: entry.dependencies,
      devDependencies: {},
      peerDependencies: entry.peerDependencies
    });
  }

  // Ensure root workspace exists
  if (!workspaces.some((w) => w.path === ".")) {
    workspaces.unshift({
      path: ".",
      name: "root",
      dependencies: {},
      devDependencies: {},
      peerDependencies: {}
    });
  }

  const patches = detectPatches(content);

  // Determine yarn version based on metadata version
  const isYarn4 =
    parseInt(metadataVersion) >= 8 || filename.startsWith("yarn4");
  const yarnVersion = isYarn4 ? "yarn@4.1.0" : "yarn@3.6.0";

  // Workspace globs derived from workspace paths
  const workspacePaths = workspaces
    .filter((w) => w.path !== ".")
    .map((w) => w.path);
  const workspaceGlobs = deriveWorkspaceGlobs(workspacePaths);

  return {
    filename,
    filepath,
    packageManager: "yarn-berry",
    lockfileName: "yarn.lock",
    frozenInstallCommand: ["yarn", "install", "--immutable"],
    workspaces,
    packageManagerVersion: yarnVersion,
    hasPatches: patches.hasPatches,
    patchFiles: patches.patchFiles,
    lockfileVersion: metadataVersion,
    rootExtras: {
      workspaces: workspaceGlobs
    }
  };
}

/**
 * Given a list of workspace paths like ["packages/a", "packages/b", "apps/web"],
 * derive workspace globs like ["packages/*", "apps/*"].
 */
function deriveWorkspaceGlobs(paths: string[]): string[] {
  const globs = new Set<string>();
  for (const p of paths) {
    const parts = p.split("/");
    if (parts.length === 2) {
      globs.add(`${parts[0]}/*`);
    } else if (parts.length === 1) {
      globs.add(p);
    } else {
      // Deeper nesting - use the first two levels
      globs.add(`${parts[0]}/${parts[1]}/*`);
    }
  }
  return Array.from(globs);
}
