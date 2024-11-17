import got from "got";
import * as Got from "got";
import { describe, it, expect, jest } from "@jest/globals";
import { isUrlOk, getRepoInfo, hasRepo } from "../src/examples";

jest.mock<typeof import("got")>("got", () => ({
  __esModule: true,
  ...jest.requireActual("got"),
}));

describe("examples", () => {
  describe("isUrlOk", () => {
    it("returns true if url returns 200", async () => {
      const mockGot = jest
        .spyOn(got, "head")
        .mockReturnValue({ statusCode: 200 } as any);

      const url = "https://github.com/vercel/turborepo/";
      const result = await isUrlOk(url);
      expect(result).toBe(true);

      expect(mockGot).toHaveBeenCalledWith(url);
      mockGot.mockRestore();
    });

    it("returns false if url returns status != 200", async () => {
      const mockGot = jest
        .spyOn(got, "head")
        .mockReturnValue({ statusCode: 401 } as any);

      const url = "https://not-github.com/vercel/turborepo/";
      const result = await isUrlOk(url);
      expect(result).toBe(false);

      expect(mockGot).toHaveBeenCalledWith(url);
      mockGot.mockRestore();
    });
  });

  describe("getRepoInfo", () => {
    it.each([
      {
        repoUrl: "https://github.com/vercel/turborepo/",
        examplePath: undefined,
        defaultBranch: "main",
        expectBranchLookup: true,
        expected: {
          username: "vercel",
          name: "turborepo",
          branch: "main",
          filePath: "",
        },
      },
      {
        repoUrl:
          "https://github.com/vercel/turborepo/tree/canary/examples/kitchen-sink",
        examplePath: undefined,
        defaultBranch: "canary",
        expectBranchLookup: false,
        expected: {
          username: "vercel",
          name: "turborepo",
          branch: "canary",
          filePath: "examples/kitchen-sink",
        },
      },
      {
        repoUrl: "https://github.com/vercel/turborepo/tree/tek/test-branch/",
        examplePath: "examples/basic",
        defaultBranch: "canary",
        expectBranchLookup: false,
        expected: {
          username: "vercel",
          name: "turborepo",
          branch: "tek/test-branch",
          filePath: "examples/basic",
        },
      },
    ])(
      "retrieves repo info for $repoUrl and $examplePath",
      async ({
        repoUrl,
        examplePath,
        defaultBranch,
        expectBranchLookup,
        expected,
      }) => {
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
    it.each([
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
    ])(
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
