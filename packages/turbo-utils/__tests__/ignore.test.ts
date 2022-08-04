import { getScopeFromArgs } from "../src";

describe("turbo-utils", () => {
  describe("getScopeFromArgs()", () => {
    it("should return null scope and empty context when no args are provided", async () => {
      const { scope, context } = getScopeFromArgs({ args: [] });
      expect(scope).toBeNull();
      expect(context).toEqual({});
    });

    it("should return argument scope and empty context with scope", async () => {
      const { scope, context } = getScopeFromArgs({ args: ["./"] });
      expect(scope).toEqual("./");
      expect(context).toEqual({});
    });

    it("used the correct argument when multiple are provided", async () => {
      const { scope, context } = getScopeFromArgs({ args: ["./", "../../"] });
      expect(scope).toEqual("./");
      expect(context).toEqual({});
    });
  });
});
