import path from "node:path";
import { randomUUID } from "node:crypto";
import { rm } from "node:fs/promises";
import {
  mkdirSync,
  existsSync,
  copySync,
  writeFileSync,
  readFileSync,
} from "fs-extra";
import yaml from "js-yaml";
import { parse as JSON5Parse } from "json5";

interface SetupTextFixtures {
  directory: string;
  test?: string;
  options?: {
    emptyFixture?: boolean;
  };
}

export function setupTestFixtures({
  directory,
  test = "",
  options = {},
}: SetupTextFixtures) {
  const fixtures: Array<string> = [];
  const parentDirectory = path.join(directory, test ? test : randomUUID());

  afterEach(async () => {
    await Promise.all(
      fixtures.map((fixture) =>
        rm(fixture, {
          retryDelay: 50,
          maxRetries: 5,
          recursive: true,
          force: true,
        })
      )
    );
  });

  afterAll(async () => {
    await rm(parentDirectory, {
      retryDelay: 50,
      maxRetries: 5,
      recursive: true,
      force: true,
    });
  });

  const useFixture = ({ fixture }: { fixture: string }) => {
    const directoryName = randomUUID();
    const testDirectory = path.join(parentDirectory, directoryName);
    if (!existsSync(testDirectory)) {
      mkdirSync(testDirectory, { recursive: true });
    }
    // keep track of it
    fixtures.push(testDirectory);

    // copy fixture to test directory
    if (!options.emptyFixture) {
      const fixturePath = path.join(directory, "__fixtures__", test, fixture);
      copySync(fixturePath, testDirectory, {
        recursive: true,
      });
    }

    const getFilePath = (filename: string) => {
      return path.isAbsolute(filename)
        ? filename
        : path.join(testDirectory, filename);
    };

    const readGenerator = (method: (filePath: string) => unknown) => {
      return <T>(filename: string) => {
        try {
          return method(getFilePath(filename)) as T;
        } catch (e) {
          return undefined;
        }
      };
    };

    const write = (
      filename: string,
      content: string | NodeJS.ArrayBufferView
    ) => {
      writeFileSync(getFilePath(filename), content);
    };

    const exists = (filename: string): boolean => {
      return existsSync(getFilePath(filename));
    };

    const read = readGenerator((filePath) => readFileSync(filePath, "utf8"));
    const readJson = readGenerator((filePath) =>
      JSON5Parse(readFileSync(filePath, "utf8"))
    );
    const readYaml = readGenerator((filePath) =>
      yaml.load(readFileSync(filePath, "utf8"))
    );

    return {
      root: testDirectory,
      read,
      readJson,
      readYaml,
      write,
      exists,
      directoryName,
    };
  };

  return { useFixture };
}
