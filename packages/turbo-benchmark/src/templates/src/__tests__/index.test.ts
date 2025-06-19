import { sum } from "../.";
import { describe, it, expect } from "@jest/globals";

describe("Hello", () => {
  it("renders without crashing", () => {
    expect(sum(1, 2)).toEqual(3);
  });
});
