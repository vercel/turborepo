import { encode } from '../qs';

export function buildUrl(
  pathExpression: string,
  params: Record<string, unknown>,
): string {
  let path = pathExpression;
  let query: Record<string, unknown> | null = null;

  for (const key in params) {
    const placeholder = `{${key}}`;
    if (path.includes(placeholder)) {
      const value = String(params[key] ?? '');
      path = path.replaceAll(placeholder, () => encodeURIComponent(value));
    } else {
      if (!query) {
        query = {};
      }
      query[key] = params[key];
    }
  }
  return query ? `${path}${encode(query)}` : path;
}
