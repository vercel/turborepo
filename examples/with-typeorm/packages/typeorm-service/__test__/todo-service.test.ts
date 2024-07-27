import { suite, expect, test, beforeEach, vi } from "vitest";
import { TodoService } from "../src/domain/todo/todo.service";
import { TodoRepository } from "../src/domain/todo/todo.repository";
import { Todo } from "../src/domain/todo/todo.entity";

suite("TodoService Unit Tests", () => {
  let todoService: TodoService;
  let mockTodoRepo: Partial<TodoRepository>;
  let mockTodo: Todo;

  beforeEach(() => {
    const now = new Date().toString();
    mockTodo = {
      id: 1,
      content: "Hello World",
      complete: false,
      createdAt: now,
      updatedAt: now,
    } as Todo;
    mockTodoRepo = {
      findById: vi.fn().mockResolvedValue(mockTodo),
      findAll: vi.fn().mockResolvedValue([mockTodo]),
      delete: vi.fn().mockResolvedValue({ affected: 1 }),
      insert: vi.fn().mockResolvedValue(mockTodo),
      update: vi.fn().mockResolvedValue({ ...mockTodo, complete: true }),
    };
    todoService = new TodoService(mockTodoRepo as TodoRepository);
  });

  test("Insert", async () => {
    const newTodo = await todoService.add("Hello World");

    expect(newTodo.content).toBe("Hello World");
    expect(newTodo.complete).toBeFalsy();
    expect(mockTodoRepo.insert).toHaveBeenCalledWith("Hello World");
  });

  test("Select", async () => {
    const todo = await todoService.findById(1);

    expect(todo?.content).toBe("Hello World");
    expect(mockTodoRepo.findById).toHaveBeenCalledWith(1);
  });

  test("Update", async () => {
    await todoService.complete(1);

    expect(mockTodoRepo.update).toHaveBeenCalledWith(1);
  });

  test("Delete", async () => {
    await todoService.deleteById(1);

    expect(mockTodoRepo.delete).toHaveBeenCalledWith(1);
  });

  test("Find All", async () => {
    const todoList = await todoService.findAll();

    expect(todoList).toEqual([mockTodo]);
    expect(mockTodoRepo.findAll).toHaveBeenCalled();
  });
});
