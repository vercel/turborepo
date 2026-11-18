import { request } from '../client';
import { fillPath } from './path';

type Verb = 'get' | 'post' | 'delete';

type Prepared<Payload> = {
  target: string;
  values: Record<string, unknown>;
  payload: Payload | undefined;
};

function prepare<Payload, Extra>(
  template: string,
  verb: Verb,
  values: object,
  payloadOrExtra: Payload | Extra | undefined,
): Prepared<Payload> {
  const plain = values as Record<string, unknown>;
  const payload = verb === 'get' ? undefined : (payloadOrExtra as Payload);
  return { target: fillPath(template, plain), values: plain, payload };
}

export function callPath<T extends string, V extends 'get' | 'delete'>(
  template: T,
  verb: V,
  values: object,
): Promise<unknown>;

export function callPath<T extends string, V extends 'post'>(
  template: T,
  verb: V,
  values: object,
  payload: { [key: string]: unknown },
): Promise<unknown>;

export async function callPath<T extends string, V extends Verb>(
  template: T,
  verb: V,
  values: object,
  payloadOrExtra?: { [key: string]: unknown } | Set<string>,
): Promise<unknown> {
  const {
    target,
    values: plain,
    payload,
  } = prepare<{ [key: string]: unknown }, Set<string>>(template, verb, values, payloadOrExtra);

  return request(target, {
    label: template,
    method: verb.toUpperCase(),
    body: payload ?? plain,
  });
}
