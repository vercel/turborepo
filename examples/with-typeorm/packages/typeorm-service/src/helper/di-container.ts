import { AppDataSource } from "../orm-config";
import { reflectFactory } from "./reflect-factory";

type Class<T = any> = { new (...args: any[]): T };

const ProxyForDatabaseInitialize = <T extends object>(obj: T): T => {
  return new Proxy(obj, {
    get(t, k) {
      const value = t[k as keyof typeof t];
      if (typeof value != "function") return value;

      return new Proxy(value, {
        apply(fn, self, args) {
          const result = fn.apply(self, args);
          if (!AppDataSource.isInitialized && result instanceof Promise)
            return result.catch(() =>
              AppDataSource.initialize().then(() => fn.apply(self, args)),
            );
          return result;
        },
      });
    },
  });
};

const Container = reflectFactory("dependency-container");

const InjectAbleStorage = reflectFactory<Function | true>("injectable");

export const inject = <T>(Target: Class<T>): T => {
  const use = InjectAbleStorage.get(Target);
  if (!use) throw new Error(`${Target.name || "Target"} is not injectable`);
  const dependencise: Class[] =
    Reflect.getMetadata("design:paramtypes", Target) || [];
  const args: unknown[] = dependencise.map(
    (C: Class) => Container.get(C) ?? inject(C),
  );
  const Component = new Target(...args);
  Container.set(Target, typeof use == "function" ? use(Component) : Component);
  return Container.get(Target) as T;
};

export function Repository(clazz: Class) {
  InjectAbleStorage.set(clazz, ProxyForDatabaseInitialize);
}

export function InjectAble(clazz: Class) {
  InjectAbleStorage.set(clazz, true);
}
