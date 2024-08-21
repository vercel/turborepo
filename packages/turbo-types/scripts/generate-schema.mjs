#!/usr/bin/env node

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { createGenerator } from "ts-json-schema-generator";

const __dirname = new URL(".", import.meta.url).pathname;

const generator = createGenerator({
  path: join(__dirname, "../src/index.ts"),
  tsconfig: join(__dirname, "../tsconfig.json"),
  type: "Schema",
});

const schemaV1 = JSON.stringify(generator.createSchema("SchemaV1"), null, 2);
writeFileSync("schemas/schema.v1.json", schemaV1);

const schemaV2 = JSON.stringify(generator.createSchema("Schema"), null, 2);
writeFileSync("schemas/schema.v2.json", schemaV2);
writeFileSync("schemas/schema.json", schemaV2);
