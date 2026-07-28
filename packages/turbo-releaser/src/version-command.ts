import { readFile, writeFile } from "node:fs/promises";
import semver from "semver";

export async function updateVersion({
  versionPath,
  increment,
  tagOverride
}: {
  versionPath: string;
  increment: semver.ReleaseType;
  tagOverride?: string;
}) {
  const [currentVersion] = (await readFile(versionPath, "utf8")).split("\n");
  const identifier = increment.startsWith("pre") ? "canary" : "latest";
  const version = semver.inc(currentVersion, increment, identifier);
  if (!version) {
    throw new Error(`Unable to increment invalid version: ${currentVersion}`);
  }

  const parsed = semver.parse(version);
  const npmTag = tagOverride || parsed?.prerelease[0] || "latest";
  await writeFile(versionPath, `${version}\n${npmTag}\n`);

  console.log(`New version: ${version}`);
  return { version, npmTag };
}
