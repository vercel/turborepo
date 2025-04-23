import path from "node:path";
import fs from "fs-extra";
import type { TransformerResults } from "../runner";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";
import type { Transformer, TransformerArgs } from "../types";

// transformer details
const TRANSFORMER = "update-schema-json-url";
const DESCRIPTION =
  'Update the "$schema" property in turbo.json from "https://turborepo.com/schema.v1.json" to "https://turborepo.com/schema.v2.json"';
const INTRODUCED_IN = "2.0.0";

/**
 * Updates the schema URL in a turbo.json file from v1 to the current version
 */
function updateSchemaUrl(content: string): string {
  return content.replace(
    "https://turborepo.com/schema.v1.json",
    "https://turborepo.com/schema.v2.json"
  );
}

export function transformer({
  root,
  options,
}: TransformerArgs): TransformerResults {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options,
  });

  log.info('Updating "$schema" property in turbo.json...');
  const turboConfigPath = path.join(root, "turbo.json");

  if (!fs.existsSync(turboConfigPath)) {
    return runner.abortTransform({
      reason: `No turbo.json found at ${root}. Is the path correct?`,
    });
  }

  try {
    // Read turbo.json as string to preserve formatting
    const turboConfigContent = fs.readFileSync(turboConfigPath, "utf8");

    // Check if it has the v1 schema URL
    if (turboConfigContent.includes("https://turborepo.com/schema.v1.json")) {
      // Replace the v1 schema URL with the current one
      const updatedContent = updateSchemaUrl(turboConfigContent);

      // Write the updated content back to the file
      runner.modifyFile({
        filePath: turboConfigPath,
        before: turboConfigContent,
        after: updatedContent,
      });

      log.info('Updated "$schema" property in turbo.json');
    } else {
      log.info("No v1 schema URL found in turbo.json. Skipping update.");
    }
  } catch (err) {
    return runner.abortTransform({
      reason: `Error updating schema URL in turbo.json: ${String(err)}`,
    });
  }

  return runner.finish();
}

const transformerMeta: Transformer = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer,
  idempotent: true,
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
