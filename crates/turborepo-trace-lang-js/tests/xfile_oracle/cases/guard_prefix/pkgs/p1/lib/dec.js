// Strings made at run time: no literal names a module, so only the test in front of a load
// can bound it.
export const dec = (s) => Buffer.from(s, 'hex').toString();
