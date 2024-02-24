import { file, file2 } from "package";

it("should ignore the package", () => {
  expect(file).toBe({});
  expect(file2).toBe({});
});
