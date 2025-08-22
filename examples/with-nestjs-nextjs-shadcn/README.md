# Turborepo Template: Next.js + NestJS + ShadCN

A modern, full-stack monorepo template built with Turborepo, featuring Next.js frontend, NestJS backend, and beautiful ShadCN UI components.

## 🚀 Features

- **Frontend**: Next.js 15 with App Router, TypeScript, and Tailwind CSS
- **Backend**: NestJS with TypeScript, decorators, and dependency injection
- **UI**: ShadCN UI components with Lucide React icons
- **Monorepo**: Turborepo for fast, incremental builds and caching
- **Shared**: ESLint, TypeScript, and Jest configurations
- **Package Manager**: pnpm with workspace support

## 📁 Project Structure

```
├── apps/
│   ├── web/          # Next.js frontend
│   └── api/          # NestJS backend
├── packages/
│   ├── ui/           # ShadCN UI components
│   ├── api/          # Shared DTOs and entities
│   ├── eslint-config/ # Shared ESLint config
│   ├── typescript-config/ # Shared TypeScript config
│   └── jest-config/  # Shared Jest config
```

## 🛠️ Getting Started

### Prerequisites

- Node.js 20+
- pnpm 8+

### Installation

```bash
# Clone the repository
git clone <your-repo-url>
cd examples/with-nestjs-nextjs-shadcn

# Install dependencies
pnpm install

# Start development servers
pnpm dev
```

### Development

```bash
# Start all apps in development mode
pnpm dev

# Build all apps and packages
pnpm build

# Lint all apps and packages
pnpm lint

# Type check all apps and packages
pnpm typecheck
```

### Adding ShadCN Components

```bash
cd apps/web
npx shadcn@latest add <component-name>
```

## 🎨 UI Components

This template includes several ShadCN components ready to use:

- **Button** - Various sizes and variants
- **Card** - Content containers with header, content, and footer
- **Input** - Form input fields
- **Badge** - Status indicators and labels

## 🔧 Configuration

### Turborepo

The project uses Turborepo for build orchestration with:

- Incremental builds
- Intelligent caching
- Parallel execution
- Shared configurations

### TypeScript

Shared TypeScript configurations for:

- Next.js apps
- NestJS apps
- React libraries

### ESLint

Shared ESLint configurations for:

- Next.js apps
- NestJS apps
- React libraries

## 📦 Packages

### `@repo/ui`

Shared UI components built with ShadCN and Tailwind CSS.

```tsx
import { Button } from "@repo/ui/components/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@repo/ui/components/card";
```

### `@repo/api`

Shared DTOs and entities for the NestJS backend.

```typescript
import { CreateLinkDto, UpdateLinkDto, Link } from "@repo/api";
```

## 🚀 Deployment

### Frontend (Next.js)

Deploy to Vercel, Netlify, or any static hosting platform.

### Backend (NestJS)

Deploy to Railway, Render, or any Node.js hosting platform.

## 📝 License

MIT
