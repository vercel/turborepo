import "reflect-metadata";
import { DataSource } from "typeorm";
import { Todo } from "./domain/todo/todo.entity";

export const AppDataSource = new DataSource({
  type: "mysql",
  host: "localhost",
  port: 3306,
  username: "root",
  password: "1234",
  database: "test_app",
  synchronize: true,
  logging: process.env.NODE_ENV === "development",
  entities: [Todo],
});
