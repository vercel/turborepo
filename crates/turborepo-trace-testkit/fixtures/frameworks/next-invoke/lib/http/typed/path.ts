import { toQuery } from '../query';

export function fillPath(template: string, values: Record<string, unknown>): string {
  let out = template;
  let rest: Record<string, unknown> | undefined;

  for (const name in values) {
    const slot = `{${name}}`;
    if (out.includes(slot)) {
      const text = String(values[name] ?? '');
      out = out.replaceAll(slot, () => encodeURIComponent(text));
    } else {
      rest = rest ?? {};
      rest[name] = values[name];
    }
  }
  return rest ? `${out}${toQuery(rest)}` : out;
}
