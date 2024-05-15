import { suite, expect, test } from "vitest";
import "reflect-metadata";
import { TodoService } from "../src/domain/todo/todo.service";
import { inject } from "../src/helper/di-container";
import { type Todo } from "../src/domain/todo/todo.entity";

const todoService = inject(TodoService);

suite("Todo", () => {
  let id: Todo["id"];

  test("Insert", async () => {
    const newTodo = await todoService.add("Hello World");

    id = newTodo.id;

    expect(newTodo.complete).toBeFalsy();
  });

  test("Select", async () => {
    const todo = await todoService.findById(id);
    expect(todo?.content).toBe("Hello World");
  });

  test("Update", async () => {
    await todoService.complete(id);

    const todo = await todoService.findById(id);

    expect(todo?.complete).toBeTruthy();
  });
  test("Delete", async () => {
    await todoService.deleteById(id);

    const todo = await todoService.findById(id);

    expect(todo).toBeNull();
  });

  test.skip("Delete All", async () => {
    const list = await todoService.findAll();
    await Promise.all(list.map((todo) => todoService.deleteById(todo.id)));

    const newList = await todoService.findAll();

    expect(newList.length).toBeFalsy();
  });
});
