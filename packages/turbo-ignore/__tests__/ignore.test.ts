import { getComparison } from "../src/getComparison";

describe("turbo-ignore", () => {
  describe("getComparison()", () => {
    it("uses headRelative comparison when not running Vercel CI", async () => {
      expect(getComparison()).toMatchInlineSnapshot(`
        Object {
          "ref": "HEAD^",
          "type": "headRelative",
        }
      `);
    });
    it("returns null when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA", async () => {
      process.env.VERCEL = "1";
      expect(getComparison()).toBeNull();
    });

    it("used previousDeploy when running in Vercel CI with VERCEL_GIT_PREVIOUS_SHA", async () => {
      process.env.VERCEL = "1";
      process.env.VERCEL_GIT_PREVIOUS_SHA = "mygitsha";
      expect(getComparison()).toMatchInlineSnapshot(`
        Object {
          "ref": "mygitsha",
          "type": "previousDeploy",
        }
      `);
    });
  });
});
