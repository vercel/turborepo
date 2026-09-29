import { cp, mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const packageName = "@ai-sdk/harness-acp";
const packageRoot = dirname(
  fileURLToPath(import.meta.resolve(`${packageName}/package.json`))
);
const target = join(
  process.cwd(),
  ".output/server/node_modules",
  packageName,
  "dist/bridge"
);

await mkdir(dirname(target), { recursive: true });
await cp(join(packageRoot, "dist/bridge"), target, { recursive: true });
