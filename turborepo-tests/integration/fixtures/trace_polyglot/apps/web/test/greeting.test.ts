import { expect, test } from "vitest";
import { greeting } from "../src/greeting";

test("greets", () => {
  expect(greeting("Ada")).toBe("Hello, Ada!");
});
