import { Repository } from "../../helper/di-container";
import { AppDataSource } from "../../orm-config";
import { Todo } from "./todo.entity";

@Repository
export class TodoRepository {
  private todoRepo = AppDataSource.getRepository(Todo);

  async findById(id: Todo["id"]) {
    return this.todoRepo.findOneBy({
      id,
    });
  }

  async findAll() {
    return this.todoRepo.find({});
  }

  async insert(content: Todo["content"]) {
    return this.todoRepo.save({ content, complete: false });
  }
  async update(id: Todo["id"]) {
    return this.todoRepo.update(id, {
      complete: true,
    });
  }
  async delete(todo: Todo["id"]) {
    return this.todoRepo.delete(todo);
  }
}
