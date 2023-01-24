// ≫
const TURBO_IGNORE_PREFIX = "\u226B  ";

function info(...args: any[]) {
  console.log(TURBO_IGNORE_PREFIX, ...args);
}

function error(...args: any[]) {
  console.error(TURBO_IGNORE_PREFIX, ...args);
}

function warn(...args: any[]) {
  console.warn(TURBO_IGNORE_PREFIX, ...args);
}

export { info, warn, error };
