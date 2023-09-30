// import * as Got from "got";
import { test, mock } from "node:test";
import assert from "node:assert";
import got from "got";
// import * as Got from "got";
import { isUrlOk, getRepoInfo, hasRepo } from "../src/examples";

// test("returns true if url returns 200", async (t) => {
//   const mockGot = t.mock.method(got, "head", () => {
//     return { statusCode: 200 };
//   });

//   const url = "https://github.com/vercel/turbo/";
//   const result = await isUrlOk(url);

//   assert.strictEqual(result, true);
//   assert.strictEqual(mockGot.mock.calls.length, 1);
//   assert.strictEqual(mockGot.mock.calls[0].arguments[0], url);
//   mock.reset();
// });




// jest.mock("got", () => ({
//   __esModule: true,
//   ...jest.requireActual("got"),
// }));

describe("examples", () => {
  describe("isUrlOk", () => {
    it("returns true if url returns 200", async () => {
        const mockGot = t.mock.method(got, "head", () => {
          return { statusCode: 200 };
        });

      const url = "https://github.com/vercel/turbo/";
      const result = await isUrlOk(url);
      assert.equal(result, true);

      assert.strictEqual(result, true);
      assert.strictEqual(mockGot.mock.calls.length, 1);
      assert.strictEqual(mockGot.mock.calls[0].arguments[0], url);
      mock.reset();
    });

    it("returns false if url returns status != 200", async () => {
        const mockGot = t.mock.method(got, "head", () => {
          return { statusCode: 401 };
        });

      const url = "https://not-github.com/vercel/turbo/";
      const result = await isUrlOk(url);
      assert.equal(result, false);

      assert.strictEqual(result, true);
      assert.strictEqual(mockGot.mock.calls.length, 1);
      assert.strictEqual(mockGot.mock.calls[0].arguments[0], url);
      mock.reset();
    });
  });

  describe("getRepoInfo", () => {
    const getRepoInfoTestCases = [
      {
        repoUrl: "https://github.com/vercel/turbo/",
        examplePath: undefined,
        defaultBranch: "main",
        expectBranchLookup: true,
        expected: {
          username: "vercel",
          name: "turbo",
          branch: "main",
          filePath: "",
        },
      },
      {
        repoUrl:
          "https://github.com/vercel/turbo/tree/canary/examples/kitchen-sink",
        examplePath: undefined,
        defaultBranch: "canary",
        expectBranchLookup: false,
        expected: {
          username: "vercel",
          name: "turbo",
          branch: "canary",
          filePath: "examples/kitchen-sink",
        },
      },
      {
        repoUrl: "https://github.com/vercel/turbo/tree/tek/test-branch/",
        examplePath: "examples/basic",
        defaultBranch: "canary",
        expectBranchLookup: false,
        expected: {
          username: "vercel",
          name: "turbo",
          branch: "tek/test-branch",
          filePath: "examples/basic",
        },
      },
    ];

    test.each(getRepoInfoTestCases)(
      "retrieves repo info for $repoUrl and $examplePath",
      async (tc) => {
        const {
          repoUrl,
          examplePath,
          defaultBranch,
          expectBranchLookup,
          expected,
        } = tc;

        const mockGot = jest.spyOn(Got, "default").mockReturnValue({
          body: JSON.stringify({ default_branch: defaultBranch }),
        } as any);

        const url = new URL(repoUrl);
        const result = await getRepoInfo(url, examplePath);
        expect(result).toMatchObject(expected);

        if (result && expectBranchLookup) {
          expect(mockGot).toHaveBeenCalledWith(
            `https://api.github.com/repos/${result.username}/${result.name}`
          );
        }

        mockGot.mockRestore();
      }
    );
  });

  describe("hasRepo", () => {
    const hasRepoTestCases = [
      {
        repoInfo: {
          username: "vercel",
          name: "turbo",
          branch: "main",
          filePath: "",
        },
        expected: true,
        expectedUrl:
          "https://api.github.com/repos/vercel/turbo/contents/package.json?ref=main",
      },
    ];

    test.each(hasRepoTestCases)(
      "checks repo at $expectedUrl",
      async ({ expected, repoInfo, expectedUrl }) => {
        const mockGot = jest
          .spyOn(got, "head")
          .mockReturnValue({ statusCode: 200 } as any);

        const result = await hasRepo(repoInfo);
        expect(result).toBe(expected);

        expect(mockGot).toHaveBeenCalledWith(expectedUrl);
        mockGot.mockRestore();
      }
    );
  });
});
