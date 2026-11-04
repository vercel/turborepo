import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const guide = await readFile(
  new URL("../content/docs/guides/tools/shadcn-ui.mdx", import.meta.url),
  "utf8",
);

test("shadcn guide uses the latest CLI tag for init and add commands", () => {
  assert.doesNotMatch(guide, /shadcn@canary/);
  assert.match(guide, /shadcn@latest init/);
  assert.match(guide, /shadcn@latest add \[COMPONENT\]/);
});
