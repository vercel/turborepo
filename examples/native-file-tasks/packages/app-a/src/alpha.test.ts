import { appendFileSync } from "node:fs";
import { expect, test } from "bun:test";
import { alpha } from "../../shared/src/alpha.ts";

test("uses alpha", () => {
	appendFileSync(".executions", "alpha\n");
	expect(alpha).toBe(1);
});
