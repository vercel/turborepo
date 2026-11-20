// The plugin is the module a caller names; the loader calls its export with an argument
// that the plugin loads (relative to the plugin's file).
export function boot(file, spec) {
  const m = require(file);
  return m.setup(spec);
}
