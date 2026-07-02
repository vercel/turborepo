import fs from "fs/promises";
import path from "path";
import { fileURLToPath } from "url";

const workspaceRoot = fileURLToPath(new URL("../../../..", import.meta.url));
const destinationDir = path.join(process.cwd(), "coverage/merged-blob");
const workspaceDirs = ["apps", "packages"];

async function mergeBlobReports() {
  await fs.rm(destinationDir, { recursive: true, force: true });
  await fs.mkdir(destinationDir, { recursive: true });

  const copiedReports: string[] = [];

  for (const workspaceDir of workspaceDirs) {
    const workspacePath = path.join(workspaceRoot, workspaceDir);
    const packageNames = await fs.readdir(workspacePath);

    for (const packageName of packageNames) {
      const blobDir = path.join(
        workspacePath,
        packageName,
        "coverage/blob"
      );

      let blobFiles: string[];

      try {
        blobFiles = await fs.readdir(blobDir);
      } catch (error) {
        if (
          error instanceof Error &&
          "code" in error &&
          error.code === "ENOENT"
        ) {
          continue;
        }

        throw error;
      }

      for (const blobFile of blobFiles) {
        if (!blobFile.endsWith(".json")) {
          continue;
        }

        const source = path.join(blobDir, blobFile);
        const destination = path.join(
          destinationDir,
          `${workspaceDir}-${packageName}-${blobFile}`
        );

        await fs.copyFile(source, destination);
        copiedReports.push(path.relative(workspaceRoot, source));
      }
    }
  }

  if (copiedReports.length === 0) {
    throw new Error("No Vitest blob reports found. Run `pnpm test` first.");
  }

  console.log(`Collected ${copiedReports.length} Vitest blob report(s).`);
}

mergeBlobReports().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
