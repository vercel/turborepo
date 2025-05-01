import { writeFile } from "node:fs/promises";

// List of files that get overwritten during CI.
// These files have content from our closed source repos
// but we still want the site to run smoothly for
// open source contributors who don't have access.
// For this reasons, these files are stubbed in
// source control and overwritten during the build.
// Files listed below are responsible for analytics
// and cookie consent banners.
const FILES_TO_WRITE = [
  {
    path: "./lib/site-analytics/index.ts",
    envVarKey: "SITE_ANALYTICS_MODULE_CODE",
  },
  {
    path: "./lib/site-analytics/index.ts",
    envVarKey: "SITE_ANALYTICS_MODULE_CODE",
  },
];

async function modifyFiles(): Promise<void> {
  if (!process.env.CI) {
    return;
  }

  await Promise.all(
    FILES_TO_WRITE.map(async (fileConfig) => {
      try {
        console.log(`Processing file: ${fileConfig.path}`);

        // Step 1: Delete the file's contents by writing an empty string
        await writeFile(fileConfig.path, "");

        const envVarContent = process.env[fileConfig.envVarKey];
        if (!envVarContent) {
          throw new Error(`No process.env.${fileConfig.envVarKey} provided.`);
        }

        // Step 2: Write new contents to the file
        await writeFile(fileConfig.path, envVarContent);
        console.log(`New contents written to ${fileConfig.path} successfully.`);
      } catch (error: unknown) {
        const errorMessage =
          error instanceof Error ? error.message : String(error);
        console.error(`Error modifying file ${fileConfig.path}:`, errorMessage);
        process.exit(1);
      }
    })
  );
}

// Execute the function
void modifyFiles().catch((error: unknown) => {
  const errorMessage = error instanceof Error ? error.message : String(error);
  console.error("Failed to modify files:", errorMessage);
  process.exit(1);
});
