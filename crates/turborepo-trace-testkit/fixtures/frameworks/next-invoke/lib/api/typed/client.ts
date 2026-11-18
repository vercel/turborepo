import { apiFetch } from '../fetch-api';
import { buildUrl } from './url';

type ApiMethod = 'get' | 'post' | 'delete';

interface TypedArgs<Body> {
  url: string;
  params: Record<string, unknown>;
  body: Body | undefined;
}

function resolveTypedArgs<Body, Options>(
  path: string,
  method: ApiMethod,
  params: object,
  bodyOrOpts: Body | Options | undefined,
): TypedArgs<Body> {
  const body = method === 'get' ? undefined : (bodyOrOpts as Body);
  const erasedParams = params as Record<string, unknown>;
  return {
    url: buildUrl(path, erasedParams),
    params: erasedParams,
    body,
  };
}

export function typedFetch<P extends string, M extends 'get' | 'delete'>(
  path: P,
  method: M,
  params: object,
): Promise<unknown>;

export function typedFetch<P extends string, M extends 'post'>(
  path: P,
  method: M,
  params: object,
  body: Record<string, unknown>,
): Promise<unknown>;

export async function typedFetch<P extends string, M extends ApiMethod>(
  path: P,
  method: M,
  params: object,
  bodyOrOpts?: Record<string, unknown> | Map<string, string>,
): Promise<unknown> {
  const {
    url,
    params: erasedParams,
    body,
  } = resolveTypedArgs<Record<string, unknown>, Map<string, string>>(
    path,
    method,
    params,
    bodyOrOpts,
  );

  return apiFetch(url, {
    route: path,
    method: method.toUpperCase(),
    body: body ?? erasedParams,
  });
}
