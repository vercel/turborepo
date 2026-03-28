#!/usr/bin/env node

import { copyFileSync } from "node:fs";
import { join } from "node:path";

["schema.json", "schema.v1.json", "schema.v2.json"].forEach((schema) => {
  copyFileSync(
    join("node_modules/@turbo/types/schemas", schema),
    join("public", schema)
  );
});
