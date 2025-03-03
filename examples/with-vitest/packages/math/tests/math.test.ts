import { expect, test } from "vitest";
import { add } from "../src/add";
import { subtract } from "../src/subtract";

test("adds 1 + 2 to equal 3", () => {
  expect(add(1, 2)).toBe(3);
});

test("subtracts 2 - 1 to equal 1", () => {
  expect(subtract(2, 1)).toBe(1);
});
