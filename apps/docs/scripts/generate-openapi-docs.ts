#!/usr/bin/env node
/**
 * Generates MDX documentation files from the OpenAPI spec.
 *
 * Usage:
 *   node --experimental-strip-types scripts/generate-openapi-docs.ts
 *   node --experimental-strip-types scripts/generate-openapi-docs.ts --check
 *
 * The --check flag compares generated files against existing files and exits
 * with code 1 if they differ (useful for CI drift detection).
 */

import { existsSync, readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import spec from "../lib/remote-cache-openapi.json" with { type: "json" };

const OUTPUT_DIR = "content/openapi";
const CHECK_MODE = process.argv.includes("--check");

/**
 * Map operationId to the desired file name (without extension).
 * This preserves existing URL structure.
 */
const OPERATION_ID_TO_FILENAME: Record<string, string> = {
  getArtifactStatus: "status",
  artifactExists: "artifact-exists",
  downloadArtifact: "download-artifact",
  uploadArtifact: "upload-artifact",
  queryArtifacts: "artifact-query",
  recordCacheEvents: "record-events"
};

/**
 * Map tag names to folder names.
 * All endpoints go in 'artifacts/' folder to preserve existing URLs.
 */
const TAG_TO_FOLDER: Record<string, string> = {
  artifacts: "artifacts",
  analytics: "artifacts" // Keep analytics endpoints in artifacts/ folder
};

// Dynamic imports to ensure we get the right version from this package's node_modules
async function main() {
  console.log(
    CHECK_MODE
      ? "Checking OpenAPI docs for drift..."
      : "Generating OpenAPI documentation..."
  );

  // Use dynamic imports to avoid TypeScript version conflicts in the monorepo
  const { generateFilesOnly } = await import("fumadocs-openapi");
  const { createOpenAPI } = await import("fumadocs-openapi/server");

  // Create the OpenAPI server instance
  const openapi = createOpenAPI({
    input: () => ({
      "remote-cache": spec
    })
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
  } as any);

  // Define the output entry type based on fumadocs-openapi
  interface OperationEntry {
    type: "operation";
    item: {
      method: string;
      path: string;
    };
    info: {
      title: string;
    };
  }

  // Generate the files in memory
  const files = await generateFilesOnly({
    input: openapi,
    // Custom grouping: put all endpoints in artifacts/ folder
    groupBy: (entry: OperationEntry) => {
      // Extract the tag from the path - all our endpoints have tags
      const path = entry.item.path;
      if (path === "/artifacts/events") {
        return TAG_TO_FOLDER["analytics"] || "artifacts";
      }
      return "artifacts";
    },
    // Use custom file names to match existing URL structure
    name: (output: OperationEntry) => {
      // Find the operationId from the OpenAPI spec
      const method = output.item.method.toLowerCase();
      const path = output.item.path;

      // Look up operationId from spec
      const pathItem =
        spec.paths[path as keyof typeof spec.paths] ||
        spec.paths["/artifacts/{hash}" as keyof typeof spec.paths];
      if (pathItem) {
        const operation = pathItem[method as keyof typeof pathItem] as
          | { operationId?: string }
          | undefined;
        if (operation?.operationId) {
          const filename = OPERATION_ID_TO_FILENAME[operation.operationId];
          if (filename) {
            return filename;
          }
        }
      }

      // Fallback: slugify the title
      return output.info.title
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "");
    },
    // Customize frontmatter for generated pages
    frontmatter: (title: string, description: string | undefined) => ({
      title,
      description,
      full: true
    })
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
  } as any);

  if (CHECK_MODE) {
    // Compare generated files against existing files
    let hasDrift = false;

    for (const file of files) {
      const filePath = join(OUTPUT_DIR, file.path);
      const fullPath = join(process.cwd(), filePath);

      if (!existsSync(fullPath)) {
        console.error(`Missing file: ${filePath}`);
        hasDrift = true;
        continue;
      }

      const existingContent = readFileSync(fullPath, "utf-8");
      if (existingContent !== file.content) {
        console.error(`Drift detected: ${filePath}`);
        hasDrift = true;
      }
    }

    if (hasDrift) {
      console.error(
        "\nOpenAPI docs are out of sync. Run 'pnpm generate:openapi' to regenerate."
      );
      process.exit(1);
    }

    console.log("OpenAPI docs are in sync.");
  } else {
    // Write generated files to disk
    for (const file of files) {
      const filePath = join(OUTPUT_DIR, file.path);
      const fullPath = join(process.cwd(), filePath);

      // Ensure directory exists
      mkdirSync(dirname(fullPath), { recursive: true });

      writeFileSync(fullPath, file.content, "utf-8");
      console.log(`  Generated: ${filePath}`);
    }

    console.log("\nDone! Generated OpenAPI documentation.");
  }
}

main().catch((error) => {
  console.error("Error generating OpenAPI docs:", error);
  process.exit(1);
});
