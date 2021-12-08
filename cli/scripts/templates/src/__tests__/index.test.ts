import { sum } from "../.";

describe("Hello", () => {
  it("renders without crashing", () => {
    expect(sum(1, 2)).toEqual(3);
  });
});
