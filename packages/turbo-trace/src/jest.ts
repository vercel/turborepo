/** The filter module, for `filter` in a jest configuration or `--filter`. */
export const filter: string = require.resolve("./jest/filter");

function apply<T>(config: T): T {
  const user = (config as { filter?: string }).filter;
  if (user && user !== filter) {
    process.stderr.write(
      `turbo trace: the jest configuration has a filter (${user}): running every test it keeps\n`
    );
    return config;
  }
  // With few selected files jest runs them in its own process, and an
  // open handle then keeps it alive: workers are killed, this process is not.
  return { ...config, filter, forceExit: true, passWithNoTests: true };
}

/**
 * Makes jest run only the test files of the selection named by
 * `TURBO_TRACE_SELECTION`. Without the variable the configuration is returned
 * as it is. An object gives an object, a function a function and a promise a
 * promise.
 */
export function withTurboTrace<T>(config: T): T {
  if (!process.env.TURBO_TRACE_SELECTION) {
    return config;
  }
  if (typeof config === "function") {
    return (async (...args: unknown[]) => apply(await config(...args))) as T;
  }
  if (config instanceof Promise) {
    return config.then(apply) as T;
  }
  return apply(config);
}
