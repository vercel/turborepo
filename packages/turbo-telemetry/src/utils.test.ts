import { describe, test } from "node:test";
import { strict as assert } from "node:assert";
import utils from "./utils";

describe("utils", () => {
  describe("oneWayHashWithSalt", () => {
    test("should return the hashed value with salt", () => {
      const input = "a-sensitive-value";
      const salt = "private-salt";

      const result = utils.oneWayHashWithSalt({ input, salt });
      assert.equal(
        result,
        "568d39ba8435f9c37e80e01c6bb6e27d7b65b4edf837e44dee662ffc99206eec"
      );
    });

    test("should return consistent length", () => {
      const input = "a-sensitive-value";
      const salt = "private-salt";

      const result1 = utils.oneWayHashWithSalt({ input, salt });
      const result2 = utils.oneWayHashWithSalt({
        input: `${input}-${input}`,
        salt,
      });

      assert.equal(result1.length, result2.length);
    });
  });

  describe("defaultConfigPath", () => {
    test("supports overriding by env var", async () => {
      process.env.TURBO_CONFIG_DIR_PATH = "/tmp";
      const result = await utils.defaultConfigPath();
      assert.equal(result, "/tmp/turborepo/telemetry.json");
    });
  });
});
