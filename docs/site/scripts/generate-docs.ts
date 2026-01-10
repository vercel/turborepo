import {
  generateFiles,
  type OperationOutput,
  type WebhookOutput
} from "fumadocs-openapi";
import type { Document } from "fumadocs-openapi";
// @ts-expect-error - Using .ts extension for node --experimental-strip-types
import { openapi } from "../lib/openapi.ts";

const out = "./content/openapi";

// Convert camelCase/PascalCase to kebab-case
const toKebabCase = (str: string): string => {
  return str
    .replace(/([a-z])([A-Z])/g, "$1-$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1-$2")
    .toLowerCase();
};

void generateFiles({
  input: openapi,
  addGeneratedComment: true,
  output: out,
  groupBy: "tag",
  slugify: toKebabCase,
  name(output: OperationOutput | WebhookOutput, document: Document): string {
    if (output.type === "operation") {
      const pathItem = document.paths?.[output.item.path];
      const operation = pathItem?.[output.item.method];
      const operationId = operation?.operationId;

      if (operationId) {
        return toKebabCase(operationId);
      }
      // Fallback to path-based name
      return this.routePathToFilePath(output.item.path);
    }

    // webhook type
    return toKebabCase(output.item.name);
  }
});
