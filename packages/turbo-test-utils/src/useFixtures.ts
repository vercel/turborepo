import { v4 as uuidv4 } from "uuid";
import path from "path";
import fs from "fs-extra";

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
    const testDirectory = path.join(parentDirectory, uuidv4());
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
      try {
        return fs.readFileSync(path.join(testDirectory, filename), "utf8");
      } catch (e) {
        return undefined;
      }
    };

    const readJson = (filename: string) => {
      try {
        return fs.readJSONSync(path.join(testDirectory, filename), "utf8");
      } catch (e) {
        return undefined;
      }
    };

    return { root: testDirectory, read, readJson };
  };

  return { useFixture };
}
