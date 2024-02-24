import { file, file2 } from "package";

it("should ignore the package", async () => {
  await expect(file).resolves.toBe({});
  expect(file2).toBe({});
});
