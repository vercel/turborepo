import fs from "node:fs";
import path from "node:path";
import { expect, test } from "vitest";

const rates = JSON.parse(
  fs.readFileSync(path.join(__dirname, "../../../shared/rates.json"), "utf8"),
);

test("knows the euro", () => {
  expect(rates.EUR).toBeGreaterThan(0);
});
