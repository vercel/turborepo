import { isServer } from './is-server';
import { trace } from './trace';

export interface FetchOptions {
  route?: string;
  req?: { headers: Record<string, string> };
  headers?: Record<string, string>;
  method?: string;
  body?: unknown;
}

if (!isServer()) {
  (window as any).apiFetch = apiFetch;
}

export async function apiFetch<ReturnData = any>(
  path: string,
  opts?: Omit<FetchOptions, 'route'>,
): Promise<ReturnData>;

export function apiFetch<ReturnData = any>(
  path: string,
  opts: FetchOptions = {},
): Promise<ReturnData> {
  // Relative URLs are not accepted on the server.
  const base = isServer()
    ? getServerBase(opts.req)
    : process.env.CLIENT_API_BASE || '';

  const isTestServer = isServer() ? isProtectedEnv() : false;

  // Accept a full URL or a relative path.
  const url = ((): string => {
    if (path.startsWith('/api')) {
      if (isTestServer) {
        return `${base}${path.replace('/api', '')}`;
      }
      return base + path;
    }

    if (path.startsWith('/')) {
      if (isTestServer) {
        return `${base}${path}`;
      }
      return base + `/api${path}`;
    }

    if (isTestServer && path.startsWith('https://api.example.com/')) {
      return path.replace('https://api.example.com', base);
    }

    return path;
  })();

  return trace((span) => apiFetchInternal(span, url, opts), {
    name: `lib/fetch-api ${opts.method || 'GET'} ${opts.route || url}`,
  });
}

async function apiFetchInternal<ReturnData = any>(
  span: unknown,
  url: string,
  opts: FetchOptions,
): Promise<ReturnData> {
  const headers = opts.headers || {};
  const method = opts.method || 'GET';
  let resp: Response | undefined;
  try {
    resp = await fetch(url, { ...opts, headers });
  } catch (e: unknown) {
    throw new Error(`${method} ${url} failed`);
  }
  return (await resp.json()) as ReturnData;
}

export default apiFetch;

function isProtectedEnv(): boolean {
  return process.env.NODE_ENV === 'development';
}

function getServerBase(req: FetchOptions['req']): string {
  if (req && isProtectedEnv()) {
    const host = req.headers['x-test-host'];
    const origin = host ? resolveOrigin(host) : undefined;
    if (origin) {
      return origin;
    }
  }
  return 'https://app.example.com';
}

function resolveOrigin(host: string): string | undefined {
  try {
    const url = new URL(`https://${host}`);
    return url.origin;
  } catch {
    return undefined;
  }
}
