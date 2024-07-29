const _get = <Value>(key: string, target: object) =>
  Reflect.getMetadata(key, target) as Value | undefined;

const _set = <Value>(key: string, target: object, value: Value) =>
  Reflect.defineMetadata(key, value, target);

const _has = (key: string, target: object): boolean =>
  Reflect.hasMetadata(key, target);

export const reflectFactory = <Value = unknown, Target extends object = object>(
  key: string,
) => ({
  get: (target: Target, defaultValue?: Value) =>
    _get<Value>(key, target) ?? defaultValue,
  set: (target: Target, value: Value) => _set<Value>(key, target, value),
  has: (target: Target) => _has(key, target),
});
