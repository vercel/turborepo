#!/usr/bin/env node

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createGenerator } from "ts-json-schema-generator";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const packageRoot = join(__dirname, "..", "src");

/**
 * post-process the schema recursively to:
 * 1. replace any key named `defaultValue` with `default`
 * 1. remove any backticks from the value
 * 1. attempt to parsing the value as JSON (falling back, if not)
 */
const postProcess = <T>(item: T): T => {
  if (typeof item !== "object" || item === null) {
    return item;
  }
  if (Array.isArray(item)) {
    return item.map(postProcess) as unknown as T;
  }
  return Object.fromEntries(
    Object.entries(item).map(([key, value]) => {
      if (key === "defaultValue" && typeof value === "string") {
        const replaced = value.replaceAll(/`/g, "");
        try {
          return ["default", JSON.parse(replaced)];
        } catch (e) {
          return ["default", replaced];
        }
      }
      return [key, postProcess(value)];
    })
  ) as T;
};

const create = (fileName: string, typeName: string) => {
  const generator = createGenerator({
    path: join(packageRoot, "index.ts"),
    tsconfig: join(__dirname, "../tsconfig.json"),
    type: "Schema",
    extraTags: ["defaultValue"],
  });
  const schema = postProcess(generator.createSchema(typeName));
  const filePath = join(__dirname, "..", "schemas", fileName);
  writeFileSync(filePath, JSON.stringify(schema, null, 2));
};

create("schema.v1.json", "SchemaV1");
create("schema.v2.json", "Schema");
create("schema.json", "Schema");
