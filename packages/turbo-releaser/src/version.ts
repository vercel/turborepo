import fs from "node:fs/promises";

export async function getVersionInfo(versionPath: string): Promise<{
  version: string;
  npmTag: string;
}> {
  const versionFile = await fs.readFile(versionPath, "utf-8");
  const [version, npmTag] = versionFile.trim().split("\n");
  console.log(`Version: ${version}, NPM Tag: ${npmTag}`);
  return { version, npmTag };
}
