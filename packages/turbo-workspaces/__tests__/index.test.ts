import path from "node:path";
import execa from "execa";
import * as turboUtils from "@turbo/utils";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect, jest } from "@jest/globals";
import { getWorkspaceDetails, convert } from "../src";
import { generateConvertMatrix } from "./test-utils";

jest.mock("execa", () => jest.fn());

describe("Node entrypoint", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
  });

  describe("convert", () => {
    it.each(generateConvertMatrix())(
      "detects $fixtureType project using $fixtureManager and converts to $toManager (interactive=$interactive dry=$dry install=$install)",
      async ({
        fixtureManager,
        fixtureType,
        toManager,
        interactive,
        dry,
        install,
      }) => {
        const mockedGetAvailablePackageManagers = jest
          .spyOn(turboUtils, "getAvailablePackageManagers")
          .mockResolvedValue({
            npm: "8.19.2",
            yarn: "1.22.19",
            pnpm: "7.29.1",
            bun: "1.0.1",
          });

        const { root } = useFixture({
          fixture: `./${fixtureManager}/${fixtureType}`,
        });

        // read
        const details = await getWorkspaceDetails({ root });
        expect(details.packageManager).toBe(fixtureManager);

        // convert
        const convertWrapper = () =>
          convert({
            root,
            to: toManager,
            options: { interactive, dry, skipInstall: !install },
          });

        if (fixtureManager === toManager) {
          await expect(convertWrapper()).rejects.toThrowError(
            "You are already using this package manager"
          );
        } else {
          await expect(convertWrapper()).resolves.toBeUndefined();
          // read again
          const convertedDetails = await getWorkspaceDetails({
            root,
          });
          expect(mockedGetAvailablePackageManagers).toHaveBeenCalled();

          if (dry) {
            expect(convertedDetails.packageManager).toBe(fixtureManager);
          } else {
            if (install) {
              expect(execa).toHaveBeenCalled();
            }
            expect(convertedDetails.packageManager).toBe(toManager);
          }
        }

        mockedGetAvailablePackageManagers.mockRestore();
      }
    );
  });
});
