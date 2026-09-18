import "dotenv/config";
import { defineConfig as ormConfig } from "@prisma/orm-postgres/config";
import { definePrismaConfig } from "prisma/config";

export default definePrismaConfig({
  orm: ormConfig({
    contract: "./prisma/schema.prisma",
    output: "./generated",
    db: {
      connection: process.env.DATABASE_URL!,
    },
  }),
});
