// A character of the caller's string: `.` loads the index file next to the helper.
export function load(spec) {
  return require(spec[0]);
}
