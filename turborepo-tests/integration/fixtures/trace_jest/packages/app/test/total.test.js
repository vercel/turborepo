const { total } = require("../src/total");

test("totals", () => {
  expect(total(2, 3, 1)).toBe(7);
});
