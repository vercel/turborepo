import { oneWayHashWithSalt, defaultConfigPath } from "./utils";

describe("utils", () => {
  describe("oneWayHashWithSalt", () => {
    test("should return the hashed value with salt", () => {
      const input = "a-sensitive-value";
      const salt = "private-salt";

      const result = oneWayHashWithSalt({ input, salt });
      expect(result).toMatchInlineSnapshot(
        `"568d39ba8435f9c37e80e01c6bb6e27d7b65b4edf837e44dee662ffc99206eec"`
      );
    });

    test("should return consistent length", () => {
      const input = "a-sensitive-value";
      const salt = "private-salt";

      const result1 = oneWayHashWithSalt({ input, salt });
      const result2 = oneWayHashWithSalt({ input: `${input}-${input}`, salt });

      expect(result1.length).toEqual(result2.length);
    });
  });

  describe("defaultConfigPath", () => {
    test("supports overriding by env var", async () => {
      process.env.TURBO_CONFIG_DIR_PATH = "/tmp";
      const result = await defaultConfigPath();
      expect(result).toEqual("/tmp/turborepo/telemetry.json");
    });
  });
});
