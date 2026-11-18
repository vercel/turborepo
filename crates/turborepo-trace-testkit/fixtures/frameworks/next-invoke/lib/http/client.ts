import { onServer } from './server-side';
import { withSpan } from './span';

export type RequestInit2 = {
  label?: string;
  method?: string;
  body?: unknown;
  headers?: { [name: string]: string };
  incoming?: { headers: { [name: string]: string } };
};

// Handy from the browser console.
if (!onServer()) {
  (globalThis as any).request = request;
}

export async function request<T = unknown>(path: string, init?: Omit<RequestInit2, 'label'>): Promise<T>;

export function request<T = unknown>(path: string, init: RequestInit2 = {}): Promise<T> {
  const origin = onServer() ? serverOrigin(init.incoming) : process.env.PUBLIC_ORIGIN || '';
  const direct = onServer() ? bypassesProxy() : false;

  // The route handlers live under `/api`; a direct backend has no such prefix.
  const target = ((): string => {
    if (path.startsWith('/api')) {
      if (direct) {
        return `${origin}${path.replace('/api', '')}`;
      }
      return origin + path;
    }

    if (path.startsWith('/')) {
      if (direct) {
        return `${origin}${path}`;
      }
      return origin + `/api${path}`;
    }

    if (direct && path.startsWith('https://backend.example.test/')) {
      return path.replace('https://backend.example.test', origin);
    }

    return path;
  })();

  return withSpan(`http ${init.label || target}`, (span) => send(span, target, init));
}

async function send<T = unknown>(span: unknown, target: string, init: RequestInit2): Promise<T> {
  const verb = init.method || 'GET';
  const headers = init.headers || {};
  let res: Response | undefined;
  try {
    res = await fetch(target, { ...init, headers });
  } catch (cause: unknown) {
    throw new Error(`request to ${target} (${verb}) did not complete`);
  }
  return (await res.json()) as T;
}

export default request;

function bypassesProxy(): boolean {
  return process.env.NODE_ENV !== 'production';
}

function serverOrigin(incoming: RequestInit2['incoming']): string {
  if (incoming && bypassesProxy()) {
    const forwarded = incoming.headers['x-preview-host'];
    const found = forwarded ? parseOrigin(forwarded) : undefined;
    if (found) {
      return found;
    }
  }
  return 'https://web.example.test';
}

function parseOrigin(host: string): string | undefined {
  try {
    return new URL(`https://${host}`).origin;
  } catch {
    return undefined;
  }
}
