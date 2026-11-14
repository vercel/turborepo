export function plugin(): unknown {
  return require(process.env.TOOL_PLUGIN);
}
