import { expect, test } from "vitest";
import { slugify } from "../src/slug";

test("slugifies", () => {
  expect(slugify(" Hello World ")).toBe("hello-world");
});
