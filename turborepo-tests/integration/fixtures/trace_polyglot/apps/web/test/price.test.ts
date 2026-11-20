import { expect, test } from "vitest";
import { formatPrice } from "../src/price";

test("formats cents", () => {
  expect(formatPrice(1250)).toBe("12.50");
});
