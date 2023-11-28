import { a as a1, b as b1, /*c as c1,*/ local as local1 } from "package-named";
import { a as a2, b as b2, local as local2 } from "package-star";
import {
  a as a3,
  b as b3,
  local as local3,
  outer as outer3,
} from "package-reexport";
import { outer as outer4 } from "package-reexport-unused";

it("should optimize named reexports from side effect free module", () => {
  expect(a1).toBe("a");
  expect(b1).toBe("b");
  // TODO handle renaming of exports
  // expect(c1).toBe("c");
});

it("should optimize star reexports from side effect free module", () => {
  expect(a2).toBe("a");
  expect(b2).toBe("b");
  expect(local2).toBe("local");
});

it("should optimize a used star reexport from module with side effects", () => {
  expect(a3).toBe("a");
  expect(b3).toBe("b");
  expect(local3).toBe("local");
  expect(outer3).toBe("outer");
});

it("should optimize a unused star reexport from module with side effects", () => {
  expect(outer4).toBe("outer");
});
