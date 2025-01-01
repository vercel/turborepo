import { inject, TodoService } from "@repo/typeorm-service";

const todoService = inject(TodoService);

export async function GET(_: Request, { params }: { params: { id: string } }) {
  const id = params.id;
  const todo = await todoService.findById(+id);

  return Response.json(todo);
}

export async function PUT(_: Request, { params }: { params: { id: string } }) {
  const id = params.id;
  const result = await todoService.complete(+id);
  if (!result.affected)
    return new Response("Not Found Todo", {
      status: 400,
    });
  return Response.json({ ok: "true" });
}

export async function DELETE(
  _: Request,
  { params }: { params: { id: string } },
) {
  const id = params.id;
  const result = await todoService.deleteById(+id);
  if (!result.affected)
    return new Response("Not Found Todo", {
      status: 400,
    });
  return Response.json({ ok: "true" });
}
