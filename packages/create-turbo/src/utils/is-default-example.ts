export const DEFAULT_EXAMPLES = new Set(["basic", "default"]);

export function isDefaultExample(example: string): boolean {
  return DEFAULT_EXAMPLES.has(example);
}
