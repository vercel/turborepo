import { describe, it, expect } from "vitest";
import { add, multiply, formatCurrency } from "./index";

describe("utils", () => {
  describe("add", () => {
    it("should add two numbers", () => {
      expect(add(2, 3)).toBe(5);
    });
  });

  describe("multiply", () => {
    it("should multiply two numbers", () => {
      expect(multiply(2, 3)).toBe(6);
    });
  });

  describe("formatCurrency", () => {
    it("should format USD by default", () => {
      expect(formatCurrency(1234.56)).toBe("$1,234.56");
    });
  });
});
