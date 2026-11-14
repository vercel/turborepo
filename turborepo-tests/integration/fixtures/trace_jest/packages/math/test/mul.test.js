const { mul } = require("../src/mul");

test("multiplies", () => {
  expect(mul(2, 3)).toBe(6);
});
