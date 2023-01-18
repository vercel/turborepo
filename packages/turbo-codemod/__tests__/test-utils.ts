import { v4 as uuidv4 } from "uuid";
import path from "path";
import fs from "fs-extra";

function getFixturePath({ test, fixture }: { test: string; fixture: string }) {
  const fixturesDirectory = path.join(__dirname, "__fixtures__");
  return path.join(fixturesDirectory, test, fixture);
}

/*
  To test with fixtures:
  1. create a temporary directory, x
  2. copy the target fixture to x
  3. run the test
  4. verify the output
  5. cleanup
*/
function setupTestFixtures({ test }: { test: string }) {
  const fixtures: Array<string> = [];
  const parentDirectory = path.join(__dirname, test);

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
    fs.copySync(getFixturePath({ test, fixture }), testDirectory, {
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
    }

    return { root: testDirectory, read, readJson };
  };

  return { useFixture };
}


export type SpyExit = { exit?: any };

function spyExit() {
  let spy: SpyExit = {};

  beforeEach(() => {
    spy.exit = jest
      .spyOn(process, "exit")
      .mockImplementation(() => undefined as never);
  });

  afterEach(() => {
    spy.exit.mockClear();
  });

  afterAll(() => {
    spy.exit.mockRestore();
  });

  return spy;
}

export { setupTestFixtures, spyExit };
