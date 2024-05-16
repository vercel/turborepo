import { getCjsState, getCjsState2 } from "./cjs";
import { getState, getState2 } from "./esm";

it("should not duplicate modules", () => {
  expect(getCjsState()).toBe(getCjsState2());
  expect(getState()).toBe(getState2());
});
