import { expect, test } from "vitest";
import { formatCount } from "../src/format";

test("formats", () => {
  expect(formatCount(2)).toBe("2 items");
});
