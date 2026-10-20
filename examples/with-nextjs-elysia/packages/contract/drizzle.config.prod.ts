import { defineConfig } from "drizzle-kit";

export default defineConfig({
  out: "./drizzle",
  schema: "./src/drizzle/table.schema.ts",
  dialect: "postgresql",
  casing: "snake_case",
  dbCredentials: {
   url:"[your prod database url]"
  },
});
