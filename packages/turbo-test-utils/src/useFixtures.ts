import { v4 as uuidv4 } from "uuid";
import path from "path";
import fs from "fs-extra";
import yaml from "js-yaml";

export default function setupTestFixtures({
  directory,
  test,
}: {
  directory: string;
  test: string;
}) {
  const fixtures: Array<string> = [];
  const parentDirectory = path.join(directory, test);

  afterEach(() => {
    fixtures.forEach((fixture) => {
      fs.rmSync(fixture, { recursive: true, force: true });
    });
  });

  afterAll(() => {
    fs.rmSync(parentDirectory, { recursive: true, force: true });
  });

  const useFixture = ({ fixture }: { fixture: string }) => {
    const tmpDirectory = uuidv4();
    const testDirectory = path.join(parentDirectory, tmpDirectory);
    if (!fs.existsSync(testDirectory)) {
      fs.mkdirSync(testDirectory, { recursive: true });
    }
    // keep track of it
    fixtures.push(testDirectory);

    // copy fixture to test directory

    const fixturePath = path.join(directory, "__fixtures__", test, fixture);
    fs.copySync(fixturePath, testDirectory, {
      recursive: true,
    });

    // helpers
    const read = (filename: string) => {
      const filePath = path.isAbsolute(filename)
        ? filename
        : path.join(testDirectory, filename);
      try {
        return fs.readFileSync(filePath, "utf8");
      } catch (e) {
        return undefined;
      }
    };

    const readJson = (filename: string) => {
      const filePath = path.isAbsolute(filename)
        ? filename
        : path.join(testDirectory, filename);
      try {
        return fs.readJSONSync(filePath, "utf8");
      } catch (e) {
        return undefined;
      }
    };

    const readYaml = (filename: string) => {
      const filePath = path.isAbsolute(filename)
        ? filename
        : path.join(testDirectory, filename);
      try {
        return yaml.load(fs.readFileSync(filePath, "utf8"));
      } catch (e) {
        return undefined;
      }
    };

    return { root: testDirectory, read, readJson, readYaml, tmpDirectory };
  };

  return { useFixture };
}
