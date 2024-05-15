# Turborepo with TypeORM

This is an official starter Turborepo configured with TypeORM to manage the service layer in a monorepo setup.

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `docs`: a [Next.js](https://nextjs.org/) app
- `web`: another [Next.js](https://nextjs.org/) app
- `ui`: a stub React component library shared by both `web` and `docs` applications
- `@repo/eslint-config`: `eslint` configurations (includes `eslint-config-next` and `eslint-config-prettier`)
- `@repo/typescript-config`: `tsconfig.json`s used throughout the monorepo
- `@repo/typeorm-service`: contains the service layer with TypeORM integration to manage database entities and transactions. It utilizes **dependency injection** to provide services across different applications.

## Dependency Injection

The @repo/typeorm-service package demonstrates a sophisticated setup where services are defined using TypeORM repositories and injected into Next.js apps using a custom dependency injection mechanism. This approach emphasizes a clear separation of concerns and a modular architecture.

```typescript
// packages/typeorm-service/domain/todo/todo.repository.ts
@Repository
export class TodoRepository {...}

// packages/typeorm-service/domain/todo/todo.service.ts
@InjectAble
export class TodoService {
    constructor(private todoRepo: TodoRepository) {}
    ...
}
```

```typescript
// app/page.tsx
import { inject, TodoService } from "@repo/typeorm-service";

export default async function Page(): Promise<JSX.Element> {
  const todoService = inject(TodoService);

  const todoList = await todoService.findAll();

  return ...
}

// app/api/todo/route.ts

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


```

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeORM](https://typeorm.io/) for service layer
- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting
