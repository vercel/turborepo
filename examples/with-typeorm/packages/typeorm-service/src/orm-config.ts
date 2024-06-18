import "reflect-metadata";
import { DataSource } from "typeorm";
import { Todo } from "./domain/todo/todo.entity";

export const AppDataSource = new DataSource({
  type: "mysql",
  host: "localhost",
  port: 3306,
  username: "root",
  database: "root",
  synchronize: true,
  logging: true,
  entities: [Todo],
});
