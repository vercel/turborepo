import { expect, test } from "vitest";
import { clamp } from "../src/clamp";

test("clamps", () => {
  expect(clamp(5, 0, 3)).toBe(3);
});
