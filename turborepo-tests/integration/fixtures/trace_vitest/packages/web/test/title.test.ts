import { expect, test } from "vitest";
import { titleId } from "../src/title";

test("builds an id", () => {
  expect(titleId("My Post")).toBe("title-my-post");
});
