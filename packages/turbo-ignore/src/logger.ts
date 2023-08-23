// the "≫" symbol
const TURBO_IGNORE_PREFIX = "\u226B  ";

function info(...args: Array<unknown>) {
  // eslint-disable-next-line no-console
  console.log(TURBO_IGNORE_PREFIX, ...args);
}

function error(...args: Array<unknown>) {
  // eslint-disable-next-line no-console
  console.error(TURBO_IGNORE_PREFIX, ...args);
}

function warn(...args: Array<unknown>) {
  // eslint-disable-next-line no-console
  console.warn(TURBO_IGNORE_PREFIX, ...args);
}

export { info, warn, error };
