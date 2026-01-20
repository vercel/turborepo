import { describe, it, expect } from "vitest";
import { Button, PriceDisplay } from "./index";

describe("ui", () => {
  describe("Button", () => {
    it("should render a button with label", () => {
      const result = Button({ label: "Click me" });
      expect(result).toContain("Click me");
      expect(result).toContain("<button");
    });
  });

  describe("PriceDisplay", () => {
    it("should display formatted price", () => {
      const result = PriceDisplay({ amount: 99.99 });
      expect(result).toContain("$99.99");
    });
  });
});
