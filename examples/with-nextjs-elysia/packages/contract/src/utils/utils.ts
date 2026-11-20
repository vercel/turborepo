/**
 * @see https://elysiajs.com/integrations/drizzle#utility
 */

import type { Static, TObject, TSchema } from "@sinclair/typebox";
import { Kind } from "@sinclair/typebox";
import type { Table } from "drizzle-orm";
import type { BuildSchema } from "drizzle-typebox";
import { createInsertSchema, createSelectSchema } from "drizzle-typebox";

type Mode = "select" | "insert" | undefined;

type Spread<T extends TObject | Table, M extends Mode> =
  T extends TObject<infer Fields>
    ? {
        [K in keyof Fields]: Fields[K];
      }
    : T extends Table
      ? M extends "select" | "insert"
        ? BuildSchema<M, T["_"]["columns"], undefined>["properties"]
        : Record<string, never>
      : Record<string, never>;

const isSchema = (schema: TObject | Table): schema is TObject => Kind in schema;

const toSchema = (schema: TObject | Table, mode: Mode): TObject => {
  if (isSchema(schema)) {
    return schema;
  }

  if (mode === "insert") {
    return createInsertSchema(schema);
  }

  if (mode === "select") {
    return createSelectSchema(schema);
  }

  throw new Error("Expected a TypeBox schema");
};

/**
 * Spread a Drizzle table or TypeBox schema into a plain object of fields
 */
export const spread = <T extends TObject | Table, M extends Mode>(
  schema: T,
  mode?: M
): Spread<T, M> => ({ ...toSchema(schema, mode).properties }) as Spread<T, M>;

/**
 * Spread a record of Drizzle tables or TypeBox schemas into plain objects
 *
 * If `mode` is 'insert', the schemas are optimized for inserts
 * If `mode` is 'select', the schemas are optimized for selects
 * If `mode` is undefined, the schemas are spread as-is
 */
export const spreads = <
  T extends Record<string, TObject | Table>,
  M extends Mode,
>(
  models: T,
  mode?: M
): {
  [K in keyof T]: Spread<T[K], M>;
} =>
  Object.fromEntries(
    Object.entries(models).map(([key, model]) => [key, spread(model, mode)])
  ) as {
    [K in keyof T]: Spread<T[K], M>;
  };

/**
 * Infer the static types of every TypeBox schema in a contract object
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
