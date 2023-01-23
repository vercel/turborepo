import { SpyConsole } from "./spyConsole";

export default function validateLogs(
  logs: Array<string | (() => boolean | Array<any>)>,
  mockConsole: SpyConsole["log"] | SpyConsole["error"],
  options: { prefix?: string } = {}
) {
  logs.forEach((log, idx) => {
    if (typeof log === "function") {
      const expected = log();
      expect(mockConsole).toHaveBeenNthCalledWith(
        idx + 1,
        ...(Array.isArray(expected) ? expected : [expected])
      );
    } else {
      if (options.prefix) {
        expect(mockConsole).toHaveBeenNthCalledWith(
          idx + 1,
          options.prefix,
          log
        );
      } else {
        expect(mockConsole).toHaveBeenNthCalledWith(idx + 1, log);
      }
    }
  });
}
