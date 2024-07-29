import { InjectAble } from "../../helper/di-container";
import { type Todo } from "./todo.entity";

import { TodoRepository } from "./todo.repository";

@InjectAble
export class TodoService {
  constructor(private todoRepo: TodoRepository) {}

  async findById(id: Todo["id"]) {
    return this.todoRepo.findById(id);
  }

  async findAll() {
    return this.todoRepo.findAll();
  }
  async deleteById(id: Todo["id"]) {
    return this.todoRepo.delete(id);
  }
  async add(content: Todo["content"]) {
    return this.todoRepo.insert(content);
  }
  async complete(id: Todo["id"]) {
    return this.todoRepo.update(id);
  }
}
