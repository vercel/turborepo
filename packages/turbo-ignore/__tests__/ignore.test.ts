import { getComparison } from "../utils";

describe("turbo-ignore", () => {
  describe("getComparison()", () => {
    it("should return HEAD comparison when not running in vercel CI", async () => {
      expect(getComparison()).toBe("HEAD^");
    });
    it("should allow build running in vercel CI with no VERCEL_GIT_PREVIOUS_SHA", async () => {
      const mockExit = jest
        .spyOn(process, "exit")
        .mockImplementation((code): never => {
          if (code === 1) {
            throw new Error("building");
          }
          throw new Error("ignored");
        });
      process.env.VERCEL = "1";
      expect(getComparison).toThrowError("building");
      mockExit.mockRestore();
    });

    it("should return VERCEL_GIT_PREVIOUS_SHA when present and running in vercel CI", async () => {
      const SHA = "mygitsha";
      process.env.VERCEL = "1";
      process.env.VERCEL_GIT_PREVIOUS_SHA = SHA;
      expect(getComparison()).toBe(SHA);
    });
  });
});
