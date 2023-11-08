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
        generator_name: "thisOne",
        config: "../config.ts",
        root: "../",
      },
      target: "run",
      calledWith: { config: "../config.ts", root: "../" },
    },
    {
      command: "run",
      options: {
        generator_name: "thisOne",
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
      calledWith: {
        empty: true,
        copy: false,
        showAllDependencies: false,
      },
    },
    {
      command: "workspace",
      options: {
        copy: "",
      },
      target: "workspace",
      calledWith: {
        empty: false,
        copy: true,
        showAllDependencies: false,
      },
    },
    {
      command: "workspace",
      options: {
        copy: "some-workspace",
        show_all_dependencies: false,
      },
      target: "workspace",
      calledWith: {
        copy: "some-workspace",
        empty: false,
        showAllDependencies: false,
      },
    },
    {
      command: "workspace",
      options: {
        type: "package",
        name: "cool-name",
        copy: true,
        show_all_dependencies: true,
      },
      target: "workspace",
      calledWith: {
        type: "package",
        name: "cool-name",
        copy: true,
        empty: false,
        showAllDependencies: true,
      },
    },
    {
      command: "workspace",
      options: {
        type: "package",
        name: "cool-name",
        empty: true,
        copy: "tailwind-css",
        destination: "../../",
        show_all_dependencies: true,
        example_path: "packages/cool-name",
      },
      target: "workspace",
      calledWith: {
        type: "package",
        name: "cool-name",
        empty: false,
        copy: "tailwind-css",
        destination: "../../",
        showAllDependencies: true,
        examplePath: "packages/cool-name",
      },
    },
    // different casing
    {
      command: "workspace",
      options: {
        type: "package",
        name: "cool-name",
        empty: true,
        copy: "tailwind-css",
        destination: "../../",
        "show-all-dependencies": true,
        "example-path": "packages/cool-name",
      },
      target: "workspace",
      calledWith: {
        type: "package",
        name: "cool-name",
        empty: false,
        copy: "tailwind-css",
        destination: "../../",
        showAllDependencies: true,
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
        .mockResolvedValue(undefined);

      const mockRun = jest.spyOn(run, "run").mockResolvedValue(undefined);

      await raw(command, { json: JSON.stringify(options) });

      if (target === "run") {
        expect(mockRun).toHaveBeenCalledWith(
          options.generator_name,
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
