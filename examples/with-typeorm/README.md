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
// root/packages/typeorm-service/domain/todo/todo.repository.ts
@Repository
export class TodoRepository {...}

// root/packages/typeorm-service/domain/todo/todo.service.ts
@InjectAble
export class TodoService {
    constructor(private todoRepo: TodoRepository) {}
    ...
}
```

## Example Usage of the Service Layer

This example demonstrates how to use the typeorm-service package to inject and use services within a Next.js app. The TodoService is injected into both page.tsx and API routes.

```typescript
// root/apps/docs/app/page.tsx
import { inject, TodoService } from "@repo/typeorm-service";

export default async function Page(): Promise<JSX.Element> {
  const todoService = inject(TodoService);

  const todoList = await todoService.findAll();

  return ...
}
```

In the API route file, TodoService is injected to handle GET and POST requests. The GET request returns the list of todos, while the POST request adds a new todo.

```typescript
// root/apps/web/app/api/todo/route.ts

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

## Configuring the Database

For managing the database settings such as the database type, username, password, and other configurations, refer to the orm-config.ts file located in the packages/typeorm-service/src directory. This file centralizes all database connection settings to ensure secure and efficient database management. Make sure to review and adjust these settings according to your environment to ensure optimal performance and security.

```typescript
// packages/typeorm-service/src/orm-config.ts
import { DataSource } from "typeorm";

export const AppDataSource = new DataSource({
    type: "mysql", // or your database type
    host: "localhost",
    port: 3306,
    username: "your_username",
    password: "your_password",
    database: "your_database_name",
    synchronize: true,
    logging: false,
    entities: [...],
    migrations: [...],
});

```

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeORM](https://typeorm.io/) for service layer
- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting
