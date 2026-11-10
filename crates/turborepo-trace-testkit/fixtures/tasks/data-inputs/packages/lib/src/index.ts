export function plugin(): unknown {
  return require(process.env.LIB_PLUGIN);
}
