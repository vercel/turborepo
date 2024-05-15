import { inject, type Todo, TodoService } from "@repo/typeorm-service";

const todoService = inject(TodoService);

export async function GET() {
  const list = await todoService.findAll();

  return Response.json(list);
}

export async function POST(req: Request) {
  const res: Pick<Todo, "content"> = await req.json();

  const entity = await todoService.add(res.content);

  return Response.json(entity);
}
