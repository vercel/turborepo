import { suite, test, beforeEach, expect } from "vitest";
import "reflect-metadata";
import {
  DataSource,
  Repository,
  Column,
  Entity,
  PrimaryGeneratedColumn,
} from "typeorm";

@Entity({
  name: "todo",
})
class Todo {
  @PrimaryGeneratedColumn()
  id: number;

  @Column({
    nullable: false,
    comment: "내용",
    length: 100,
  })
  content: string;

  @Column()
  complete: boolean;
}

suite("typeorm with SQL.js", () => {
  let dataSource: DataSource;
  let todoRepo: Repository<Todo>;

  beforeEach(async () => {
    dataSource = new DataSource({
      type: "sqljs",
      entities: [Todo],
      synchronize: true,
      autoSave: false,
      dropSchema: true,
    });

    await dataSource.initialize();

    todoRepo = dataSource.getRepository(Todo);
  });

  test("Insert", async () => {
    const newTodo = await todoRepo.save({
      content: "Hello World",
      complete: false,
    });
    expect(newTodo.content).toBe("Hello World");
    expect(newTodo.complete).toBeFalsy();
  });

  test("Select", async () => {
    const newTodo = await todoRepo.save({
      content: "Hello World",
      complete: false,
    });
    const todo = await todoRepo.findOneBy({ id: newTodo.id });

    expect(todo?.content).toBe("Hello World");
  });

  test("Update", async () => {
    const newTodo = await todoRepo.save({
      content: "Hello World",
      complete: false,
    });
    await todoRepo.update(newTodo.id, { complete: true });
    const todo = await todoRepo.findOneBy({ id: newTodo.id });
    expect(todo?.complete).toBeTruthy();
  });

  test("Delete", async () => {
    const newTodo = await todoRepo.save({
      content: "Hello World",
      complete: false,
    });
    await todoRepo.delete(newTodo.id);

    const exist = await todoRepo.existsBy({ id: newTodo.id });
    expect(exist).toBe(false);
  });
});
