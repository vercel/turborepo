import { raw } from "../src/commands/raw";
import * as run from "../src/commands/run";
import * as workspace from "../src/commands/workspace";

describe("raw", () => {
  const testMatrix = [
    // run
    {
      command: "run",
      options: { config: "../config.ts", root: "../" },
      target: "run",
      calledWith: { config: "../config.ts", root: "../" },
    },
    {
      command: "run",
      options: {
        "generator-name": "thisOne",
        config: "../config.ts",
        root: "../",
      },
      target: "run",
      calledWith: { config: "../config.ts", root: "../" },
    },
    {
      command: "run",
      options: {
        "generator-name": "thisOne",
        config: "../config.ts",
        root: "../",
        args: ["cool name", "packages/cool-name"],
      },
      target: "run",
      calledWith: {
        config: "../config.ts",
        root: "../",
        args: ["cool name", "packages/cool-name"],
      },
    },
    // workspace
    {
      command: "workspace",
      options: {},
      target: "workspace",
      calledWith: {},
    },
    {
      command: "workspace",
      options: {
        type: "package",
        name: "cool-name",
        copy: true,
        "show-all-dependencies": true,
      },
      target: "workspace",
      calledWith: {
        type: "package",
        name: "cool-name",
        copy: true,
        showAllDependencies: true,
      },
    },
    {
      command: "workspace",
      options: {
        type: "package",
        name: "cool-name",
        empty: true,
        destination: "../../",
        "show-all-dependencies": true,
        example: "tailwind-css",
        "example-path": "packages/cool-name",
      },
      target: "workspace",
      calledWith: {
        type: "package",
        name: "cool-name",
        empty: true,
        destination: "../../",
        showAllDependencies: true,
        example: "tailwind-css",
        examplePath: "packages/cool-name",
      },
    },
  ];
  test.each(testMatrix)(
    "$command and $options calls $target with $calledWith",
    async ({ command, options, target, calledWith }) => {
      // mock generation functions, we only care if they are called,
      // and what they are called with
      const mockWorkspace = jest
        .spyOn(workspace, "workspace")
        .mockImplementation(() => Promise.resolve(undefined));

      const mockRun = jest
        .spyOn(run, "run")
        .mockImplementation(() => Promise.resolve(undefined));

      await raw(command, { json: JSON.stringify(options) });

      if (target === "run") {
        expect(mockRun).toHaveBeenCalledWith(
          options["generator-name"],
          calledWith
        );
        expect(mockWorkspace).not.toHaveBeenCalled();
      } else {
        expect(mockWorkspace).toHaveBeenCalledWith(calledWith);
        expect(mockRun).not.toHaveBeenCalled();
      }

      mockWorkspace.mockRestore();
      mockRun.mockRestore();
    }
  );
});
