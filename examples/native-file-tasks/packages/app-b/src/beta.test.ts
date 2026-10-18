import { appendFileSync } from "node:fs";
import { expect, test } from "bun:test";
import { beta } from "../../shared/src/beta.ts";

test("uses beta", () => {
	appendFileSync(".executions", "beta\n");
	expect(beta).toBe(2);
});
