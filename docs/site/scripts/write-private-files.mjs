import { writeFile } from "node:fs/promises";

console.log(process.env);

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
    path: "./lib/site-analytics/hooks/use-consent-banner.ts",
    envVarKey: "CONSENT_HOOK_CODE",
  },
  {
    path: "./lib/site-analytics/index.ts",
    envVarKey: "SITE_ANALYTICS_MODULE_CODE",
  },
  {
    path: "./components/consent-banner/analytics-scripts.tsx",
    envVarKey: "ANALYTICS_SCRIPTS_COMPONENT_CODE",
  },
];

async function modifyFiles() {
  if (!process.env.CI) {
    return;
  }

  for (const fileConfig of FILES_TO_WRITE) {
    try {
      console.log(`Processing file: ${fileConfig.path}`);

      // Step 1: Delete the file's contents by writing an empty string
      console.log(`Deleting contents of ${fileConfig.path}...`);
      await writeFile(fileConfig.path, "");
      console.log("File contents deleted successfully.");

      const envVarContent = process.env[fileConfig.envVarKey];
      if (!envVarContent) {
        throw new Error(`No process.env.${fileConfig.envVarKey} provided.`);
      }

      // Step 2: Write new contents to the file
      console.log("Writing new contents...");
      await writeFile(fileConfig.path, envVarContent);
      console.log("New contents written successfully.");

      console.log(`File modification complete for ${fileConfig.path}!`);
    } catch (error) {
      console.error(`Error modifying file ${fileConfig.path}:`, error.message);
    }
  }
}

// Execute the function
modifyFiles();
