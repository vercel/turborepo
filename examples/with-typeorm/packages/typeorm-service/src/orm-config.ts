import "reflect-metadata";
import { DataSource } from "typeorm";
import { Todo } from "./domain/todo/todo.entity";

export const AppDataSource = new DataSource({
  type: "sqljs",
  synchronize: true,
  logging: true,
  entities: [Todo],
  autoSave: false,
  dropSchema: true,
});
