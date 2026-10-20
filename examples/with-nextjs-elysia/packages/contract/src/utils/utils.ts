/**
 * @lastModified 2025-02-04
 * @see https://elysiajs.com/recipe/drizzle.html#utility
 */

import {
  Kind,
  type Static,
  type TObject,
  type TSchema,
} from "@sinclair/typebox";
import type { Table } from "drizzle-orm";
import {
  type BuildSchema,
  createInsertSchema,
  createSelectSchema,
} from "drizzle-typebox";

type Spread<
  T extends TObject | Table,
  Mode extends "select" | "insert" | undefined,
> = T extends TObject<infer Fields>
  ? {
      [K in keyof Fields]: Fields[K];
    }
  : T extends Table
    ? Mode extends "select"
      ? BuildSchema<"select", T["_"]["columns"], undefined>["properties"]
      : Mode extends "insert"
        ? BuildSchema<"insert", T["_"]["columns"], undefined>["properties"]
        : {}
    : {};

/**
 * 将 Drizzle 模式展开为一个普通对象
 */
export const spread = <
  T extends TObject | Table,
  Mode extends "select" | "insert" | undefined,
>(
  schema: T,
  mode?: Mode
): Spread<T, Mode> => {
  const newSchema: Record<string, unknown> = {};
  let table;

  switch (mode) {
    case "insert":
    case "select":
      if (Kind in schema) {
        table = schema;
        break;
      }

      table =
        mode === "insert"
          ? createInsertSchema(schema)
          : createSelectSchema(schema);

      break;

    default:
      if (!(Kind in schema)) throw new Error("期望是一个模式");
      table = schema;
  }

  for (const key of Object.keys(table.properties))
    newSchema[key] = table.properties[key];

  return newSchema as any;
};

/**
 * 将 Drizzle 表展开为一个普通对象
 *
 * 如果 `mode` 是 'insert'，则模式将经过插入优化
 * 如果 `mode` 是 'select'，则模式将经过选择优化
 * 如果 `mode` 是未定义，模式将按原样展开，模型需要手动优化
 */
export const spreads = <
  T extends Record<string, TObject | Table>,
  Mode extends "select" | "insert" | undefined,
>(
  models: T,
  mode?: Mode
): {
  [K in keyof T]: Spread<T[K], Mode>;
} => {
  const newSchema: Record<string, unknown> = {};
  const keys = Object.keys(models);

  for (const key of keys) newSchema[key] = spread(models[key]!, mode);

  return newSchema as any;
};

/**
 * 🛠️ 自动 DTO 推导工具
 * 提取 Contract 中所有 TSchema 字段的静态类型
 */
export type InferDTO<T> = {
  [K in keyof T]: T[K] extends TSchema ? Static<T[K]> : never;
};

export function pick<T, K extends keyof T>(obj: T, keys: K[]): Pick<T, K> {
  const result = {} as Pick<T, K>;
  for (const key of keys) {
    result[key] = obj[key];
  }
  return result;
}

// 使用
// const picked = pick(original, ['id', 'name', 'email']);
// 类型自动推导为 { id: string; name: string; email: string; }
