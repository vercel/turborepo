import { a as a1, b as b1 } from "package";
import { a as a2, b as b2 } from "package2";

it("should optimize named reexports from side effect free module", () => {
  expect(a1).toBe("a");
  expect(b1).toBe("b");
});

it("should optimize star reexports from side effect free module", () => {
  expect(a2).toBe("a");
  expect(b2).toBe("b");
});
