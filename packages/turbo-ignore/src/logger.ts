// the "≫" symbol
const TURBO_IGNORE_PREFIX = "\u226B  ";

function info(...args: Array<unknown>) {
  // eslint-disable-next-line no-console -- logger
  console.log(TURBO_IGNORE_PREFIX, ...args);
}

function error(...args: Array<unknown>) {
  // eslint-disable-next-line no-console -- logger
  console.error(TURBO_IGNORE_PREFIX, ...args);
}

function warn(...args: Array<unknown>) {
  // eslint-disable-next-line no-console -- logger
  console.warn(TURBO_IGNORE_PREFIX, ...args);
}

function log(...args: Array<unknown>) {
  // eslint-disable-next-line no-console -- logger
  console.log(...args);
}

export { log, info, warn, error };
