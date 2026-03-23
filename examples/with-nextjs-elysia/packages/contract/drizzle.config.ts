import { defineConfig } from "drizzle-kit";

export default defineConfig({
  out: "./drizzle",
  schema: "./src/table.schema.ts",
  dialect: "postgresql",
  casing: "snake_case",
  dbCredentials: {
    url: "[your database url]"
  },
});
