import { describe, it, expect } from "vitest";
import { renderApp, calculateTotal } from "./index";

describe("web app", () => {
  describe("renderApp", () => {
    it("should render the app HTML", () => {
      const result = renderApp();
      expect(result).toContain("Welcome to the Web App");
      expect(result).toContain("Get Started");
    });
  });

  describe("calculateTotal", () => {
    it("should sum all items", () => {
      expect(calculateTotal([1, 2, 3, 4, 5])).toBe(15);
    });

    it("should return 0 for empty array", () => {
      expect(calculateTotal([])).toBe(0);
    });
  });
});
