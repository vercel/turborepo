import path from "node:path";
import fs from "fs-extra";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import type { PackageManager } from "@turbo/utils";
import {
  replacePackageManagerReferences,
  transform
} from "../src/transforms/update-commands-in-readme";
import type { TransformInput } from "../src/transforms/types";

function makeTransformInput(
  overrides: Partial<TransformInput["prompts"]>
): TransformInput {
  return {
    prompts: {
      projectName: "test-project",
      root: overrides.root ?? "/tmp/test",
      packageManager: overrides.packageManager ?? {
        name: "npm",
        version: "8.0.0"
      }
    }
  } as unknown as TransformInput;
}

describe("replacePackageManagerReferences", () => {
  describe("compound '<pm> run' replacements", () => {
    it("replaces 'pnpm run build' with '<selected> run build'", () => {
      const input = "Run `pnpm run build` to compile.";
      expect(replacePackageManagerReferences("npm", input)).toBe(
        "Run `npm run build` to compile."
      );
    });

    it("replaces 'npm run dev' with '<selected> run dev'", () => {
      const input = "Start with `npm run dev`.";
      expect(replacePackageManagerReferences("yarn", input)).toBe(
        "Start with `yarn run dev`."
      );
    });

    it("replaces all four package manager run commands", () => {
      const managers: Array<PackageManager> = ["pnpm", "npm", "yarn", "bun"];
      for (const pm of managers) {
        const input = `Use \`${pm} run test\` to run tests.`;
        expect(replacePackageManagerReferences("bun", input)).toBe(
          "Use `bun run test` to run tests."
        );
      }
    });
  });

  describe("bare '<pm>' replacements", () => {
    it("replaces bare 'pnpm' with '<selected>' (NOT '<selected> run')", () => {
      const input = "Run `pnpm install` first.";
      expect(replacePackageManagerReferences("npm", input)).toBe(
        "Run `npm install` first."
      );
    });

    it("replaces bare 'yarn' in 'yarn install'", () => {
      const input = "```\nyarn install\n```";
      expect(replacePackageManagerReferences("pnpm", input)).toBe(
        "```\npnpm install\n```"
      );
    });

    it("replaces 'pnpm exec' with '<selected> exec'", () => {
      const input = "`pnpm exec turbo build`";
      expect(replacePackageManagerReferences("npm", input)).toBe(
        "`npm exec turbo build`"
      );
    });

    it("does not corrupt subcommands like dlx, exec, add, init", () => {
      const subcommands = ["dlx", "exec", "add", "init", "install", "create"];
      for (const sub of subcommands) {
        const input = `\`pnpm ${sub} foo\``;
        const result = replacePackageManagerReferences("npm", input);
        expect(result).toBe(`\`npm ${sub} foo\``);
        expect(result).not.toContain("npm run");
      }
    });
  });

  describe("does not modify text outside code regions", () => {
    it("leaves prose text unchanged", () => {
      const input =
        "This project uses pnpm as its package manager. Install pnpm first.";
      expect(replacePackageManagerReferences("npm", input)).toBe(input);
    });

    it("only replaces inside backtick-delimited regions", () => {
      const input =
        "We recommend pnpm. Run `pnpm install` to get started with pnpm.";
      expect(replacePackageManagerReferences("npm", input)).toBe(
        "We recommend pnpm. Run `npm install` to get started with pnpm."
      );
    });
  });

  describe("fenced code blocks", () => {
    it("replaces inside fenced code blocks", () => {
      const input = "```sh\npnpm run build\npnpm run dev\n```";
      expect(replacePackageManagerReferences("yarn", input)).toBe(
        "```sh\nyarn run build\nyarn run dev\n```"
      );
    });

    it("handles mixed commands in a fenced block", () => {
      const input = "```\nyarn install\nyarn run build\nyarn dlx turbo\n```";
      expect(replacePackageManagerReferences("npm", input)).toBe(
        "```\nnpm install\nnpm run build\nnpm dlx turbo\n```"
      );
    });

    it("handles fenced blocks with language identifiers", () => {
      const input = "```sh\npnpm exec turbo build\n```";
      expect(replacePackageManagerReferences("npm", input)).toBe(
        "```sh\nnpm exec turbo build\n```"
      );
    });
  });

  describe("multiple code regions", () => {
    it("handles multiple inline code spans independently", () => {
      const input = "Use `pnpm build` or `yarn dev` to start.";
      expect(replacePackageManagerReferences("npm", input)).toBe(
        "Use `npm build` or `npm dev` to start."
      );
    });

    it("handles mix of fenced and inline code", () => {
      const input =
        "Run `pnpm install` then:\n\n```sh\npnpm run build\n```\n\nOr use `yarn dev`.";
      expect(replacePackageManagerReferences("bun", input)).toBe(
        "Run `bun install` then:\n\n```sh\nbun run build\n```\n\nOr use `bun dev`."
      );
    });
  });

  describe("identity replacement", () => {
    it("does not corrupt content when target matches source", () => {
      const input = "```\npnpm install\npnpm run build\n```";
      expect(replacePackageManagerReferences("pnpm", input)).toBe(input);
    });
  });

  describe("realistic README content", () => {
    it("handles the basic example README pattern", () => {
      const input = [
        "```sh",
        "npx turbo build",
        "yarn dlx turbo build",
        "pnpm exec turbo build",
        "```"
      ].join("\n");
      // npx is a separate binary, not in the replacement list.
      // yarn/pnpm are bare matches and get replaced with the target PM.
      const result = replacePackageManagerReferences("pnpm", input);
      expect(result).toBe(
        [
          "```sh",
          "npx turbo build",
          "pnpm dlx turbo build",
          "pnpm exec turbo build",
          "```"
        ].join("\n")
      );
    });

    it("replaces run commands in a Docker example README", () => {
      const input = "```\n# Install dependencies\nyarn install\n```";
      expect(replacePackageManagerReferences("pnpm", input)).toBe(
        "```\n# Install dependencies\npnpm install\n```"
      );
    });
  });
});

describe("transform", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    options: { emptyFixture: true }
  });

  it("returns not-applicable when packageManager is undefined", async () => {
    const input = makeTransformInput({ packageManager: undefined });
    const result = await transform(input);
    expect(result.result).toBe("not-applicable");
  });

  it("returns not-applicable when README.md does not exist", async () => {
    const { root } = useFixture({ fixture: "create-turbo" });
    const input = makeTransformInput({
      root,
      packageManager: { name: "npm", version: "8.0.0" }
    });
    const result = await transform(input);
    expect(result.result).toBe("not-applicable");
  });

  it("reads, transforms, and writes README.md", async () => {
    const { root } = useFixture({ fixture: "create-turbo" });
    const readmePath = path.join(root, "README.md");
    await fs.writeFile(
      readmePath,
      "Run `pnpm run build` and `pnpm install`.",
      "utf8"
    );

    const input = makeTransformInput({
      root,
      packageManager: { name: "yarn", version: "1.22.0" }
    });
    const result = await transform(input);
    expect(result.result).toBe("success");

    const content = await fs.readFile(readmePath, "utf8");
    expect(content).toBe("Run `yarn run build` and `yarn install`.");
  });

  it("returns success with correct meta", async () => {
    const { root } = useFixture({ fixture: "create-turbo" });
    await fs.writeFile(path.join(root, "README.md"), "Hello", "utf8");

    const input = makeTransformInput({
      root,
      packageManager: { name: "npm", version: "8.0.0" }
    });
    const result = await transform(input);
    expect(result).toEqual({
      result: "success",
      name: "update-commands-in-readme"
    });
  });
});
