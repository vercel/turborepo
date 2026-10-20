import path from "node:path/posix";

// Package managers gate installs of freshly published versions behind a
// release-age setting (`minimumReleaseAge` and friends). Their exclusion lists
// -- `minimumReleaseAgeExclude` for pnpm, `minimumReleaseAgeExcludes` for Bun,
// and `minimum-release-age-exclude` for npm -- are the escape hatch that lets a
// specific package skip that gate. Examples maintenance pins exact latest
// published versions instead, so the agent must never write an exclusion list
// to make an upgrade install.
const releaseAgeExcludePattern = /minimum[-_]?release[-_]?age[-_]?excludes?/i;
const releaseAgeConfigFiles = new Set([
  ".npmrc",
  ".yarnrc.yaml",
  ".yarnrc.yml",
  "bunfig.toml",
  "package.json",
  "pnpm-workspace.yaml",
  "pnpm-workspace.yml"
]);
const MAX_FINDING_LENGTH = 120;

export interface ReleaseAgeExclusionFinding {
  line: number;
  text: string;
}

export function isReleaseAgeConfigFile(filePath: string): boolean {
  return releaseAgeConfigFiles.has(path.basename(filePath));
}

export function findReleaseAgeExclusion(
  filePath: string,
  content: string
): ReleaseAgeExclusionFinding | null {
  if (!isReleaseAgeConfigFile(filePath)) {
    return null;
  }

  const lines = content.split("\n");
  for (const [index, line] of lines.entries()) {
    if (releaseAgeExcludePattern.test(line)) {
      return { line: index + 1, text: truncate(line.trim()) };
    }
  }

  return null;
}

export function assertNoReleaseAgeExclusion(
  filePath: string,
  content: string
): void {
  const finding = findReleaseAgeExclusion(filePath, content);
  if (finding === null) {
    return;
  }

  throw new Error(
    `${filePath}:${finding.line} adds a release-age exclusion (${finding.text}). Examples maintenance upgrades to the latest published versions without release-age exclusions. Remove the setting instead of excluding packages from it.`
  );
}

function truncate(value: string): string {
  return value.length <= MAX_FINDING_LENGTH
    ? value
    : `${value.slice(0, MAX_FINDING_LENGTH)}…`;
}
