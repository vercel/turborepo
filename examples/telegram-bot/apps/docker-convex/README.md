# Docker Convex Boilerplate

A self-hosted **Convex Backend-as-a-Service** boilerplate running in Docker. Unlike traditional databases where you write SQL queries or use ORMs, Convex combines your database and API layer into one unified system where your **API endpoints are defined as functions inside the database itself**.

Perfect for developers familiar with SQL/NoSQL databases who want to modernize their stack with real-time capabilities, type safety, and serverless-style functions.

## What Makes Convex Different

**Traditional Stack:**

```
Next.js App → API Routes → ORM/Query Builder → Database
```

**Convex Stack:**

```
Next.js App → Generated Client → Convex Functions (Your API + Database)
```

### Key Differences from SQL/NoSQL:

- **No separate API layer**: Your database functions ARE your API endpoints
- **Real-time by default**: All queries automatically subscribe to changes
- **Type-safe**: Full TypeScript support from database to frontend
- **Serverless functions**: Write backend logic as simple TypeScript functions

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                Docker Convex Backend-as-a-Service              │
├─────────────────────────────────────────────────────────────────┤
│  Next.js Container            │  Convex Container              │
│  ┌─────────────────────┐     │  ┌─────────────────────────┐   │
│  │ Your Frontend       │────▶│  │ convex/example.ts       │   │
│  │ - useQuery()        │     │  │ ┌─────────────────────┐ │   │
│  │ - useMutation()     │     │  │ │ export const        │ │   │
│  │ - Real-time updates │     │  │ │ listItems = query() │ │   │
│  │ - Type safety       │     │  │ │ addItem = mutation()│ │   │
│  └─────────────────────┘     │  │ │ deleteItem = ...    │ │   │
│           │                  │  │ └─────────────────────┘ │   │
│           ▼                  │  └─────────────────────────┘   │
│  ┌─────────────────────┐     │           │                     │
│  │ Generated Client    │◀────┼───────────┘                     │
│  │ - api.example.*     │     │  ┌─────────────────────────┐   │
│  │ - Type definitions  │     │  │ Docker Backend          │   │
│  │ - Real-time hooks   │     │  │ Port 3210 (API)        │   │
│  └─────────────────────┘     │  │ Port 6791 (Dashboard)   │   │
│                               │  └─────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Process Flow

### 1. `self-hosted:setup` Process

```
pnpm run self-hosted:setup
         │
         ▼
┌─────────────────────┐
│ docker:up           │ ──▶ Start containers (backend + dashboard)
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ Wait 10 seconds     │ ──▶ Let backend initialize
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ generate-admin-key  │ ──▶ Create admin credentials
└─────────────────────┘
```

### 2. `deploy-functions` Process

```
pnpm run deploy-functions
         │
         ▼
┌─────────────────────┐
│ convex dev --once   │ ──▶ Deploy functions to self-hosted instance
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ Analyze convex/     │ ──▶ Scan *.ts files (example.ts, etc.)
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ Generate _generated │ ──▶ Create API bindings & types
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ Push to backend     │ ──▶ Upload functions to Docker instance
└─────────────────────┘
```

## Table of Contents

- [Prerequisites](#prerequisites)
- [Setup Guide](#setup-guide)
  - [Installation](#1-clone-repository)
  - [Database Initialization](#2-initialize-database-automated)
  - [Database Operations](#3-database-operations)
  - [Manual Configuration](#4-manual-database-setup)
- [Database Management Commands](#database-management-commands)
- [Database Access](#database-access)
- [Troubleshooting](#troubleshooting)
- [Generated Files Explained](#generated-files-explained)

## Prerequisites

- [Docker](https://www.docker.com/get-started/) for database containerization
- [Node.js](https://nodejs.org/) (v18+) for running management tools
- [pnpm](https://pnpm.io/) (v8+) for package management

---

## Quick Start

Get your Convex database running in one command:

```bash
pnpm run self-hosted:setup
```

This will:

- Start the Docker containers
- Generate admin credentials
- Display the deployment URL, dashboard URL, and admin key
- Save the admin key to a timestamped file in `./admin-key/`

Then deploy your functions:

```bash
pnpm run deploy-functions
```

## Connecting to Your Next.js App (Docker Container)

### Option 1: Side-by-Side Containers (Recommended)

Create a `docker-compose.yml` in your main project:

```yaml
version: "3.8"
services:
  # Your Next.js frontend
  frontend:
    build: ./frontend
    ports:
      - "3000:3000"
    environment:
      - NEXT_PUBLIC_CONVEX_URL=http://convex-backend:3210
    depends_on:
      - convex-backend
    networks:
      - app-network

  # This Convex backend
  convex-backend:
    build: ./docker-convex # Path to this boilerplate
    ports:
      - "3210:3210" # API
      - "6791:6791" # Dashboard
    volumes:
      - convex-data:/app/convex-data
    networks:
      - app-network

volumes:
  convex-data:

networks:
  app-network:
    driver: bridge
```

### Option 2: Turborepo Integration

In your `turbo.json`:

```json
{
  "pipeline": {
    "dev": {
      "dependsOn": ["^build"],
      "cache": false,
      "persistent": true
    },
    "convex:dev": {
      "cache": false,
      "persistent": true
    }
  }
}
```

### Frontend Setup (Next.js)

1. **Install Convex in your Next.js project:**

   ```bash
   npm install convex
   ```

2. **Configure environment variables:**

   ```bash
   # .env.local
   NEXT_PUBLIC_CONVEX_URL=http://localhost:3210
   # For production: NEXT_PUBLIC_CONVEX_URL=http://convex-backend:3210
   ```

3. **Setup Convex provider in your app:**

   ```typescript
   // app/layout.tsx or pages/_app.tsx
   import { ConvexProvider, ConvexReactClient } from "convex/react";

   const convex = new ConvexReactClient(process.env.NEXT_PUBLIC_CONVEX_URL!);

   export default function RootLayout({ children }) {
     return <ConvexProvider client={convex}>{children}</ConvexProvider>;
   }
   ```

4. **Use in your components:**

   ```typescript
   // components/ItemList.tsx
   import { useQuery, useMutation } from "convex/react";
   import { api } from "../convex/_generated/api";

   export function ItemList() {
     const items = useQuery(api.example.listItems);
     const addItem = useMutation(api.example.addItem);
     const deleteItem = useMutation(api.example.deleteItem);

     return (
       <div>
         {items?.map((item) => (
           <div key={item._id}>
             {item.name}
             <button onClick={() => deleteItem({ id: item._id })}>
               Delete
             </button>
           </div>
         ))}
         <button onClick={() => addItem({ name: "New Item" })}>Add Item</button>
       </div>
     );
   }
   ```

## Database Setup

This project runs a Convex database instance in Docker containers, providing a complete database environment with persistence and administration capabilities.

### 1. Configure Database Environment (Optional)

For custom configuration, set up the environment files:

```bash
cp .env.docker.example .env.docker
```

**Configure Database Settings** in `.env.docker`:

- Set a secure `INSTANCE_SECRET` for database encryption
- Configure database ports if needed (defaults: 3210, 3211, 6791)
- Optionally configure storage paths and memory limits

### 2. One-Command Setup (Recommended)

Start everything with a single command:

```bash
pnpm run self-hosted:setup
```

This command will:

- Start Docker containers
- Wait for the backend to be ready
- Generate and display admin credentials
- Save credentials to `./admin-key/admin_key_[timestamp].md`

### 3. Database Operations

Start the database server:

```bash
pnpm run docker:up
```

Stop the database server:

```bash
pnpm run docker:down
```

Deploy schema changes:

```bash
pnpm run deploy-functions
```

### 4. Manual Database Setup

For manual control over the database setup:

1. Start database and create admin credentials:

   ```bash
   pnpm run self-hosted:setup-manual
   ```

2. Configure admin access in `.devcontainer/.env.local`:

   ```bash
   CONVEX_SELF_HOSTED_ADMIN_KEY=<generated-admin-key>
   ```

3. Initialize database schema:
   ```bash
   pnpm run deploy-functions
   ```

---

---

## Database Management Commands

### Quick Setup

- `pnpm run self-hosted:setup` - **One-command setup**: Start containers and generate admin key
- `pnpm run deploy-functions` - Deploy schema changes

### Core Database Operations

- `pnpm run docker:up` - Start the database server
- `pnpm run docker:down` - Stop the database server
- `pnpm run docker:logs` - View database logs

### Administration

- `pnpm run docker:generate-admin-key` - Generate new admin credentials
- `pnpm run self-hosted:setup-manual` - Manual database initialization
- `pnpm run self-hosted:reset` - Stop services and perform full Docker reset

### Maintenance

- `pnpm run docker:cleanup-admin-keys` - Remove saved admin key files
- `pnpm run docker:reset-images` - Stop Docker and prune system volumes
- `pnpm run docker:reset-full` - Combine key cleanup and image reset

> All commands use `pnpm`. Ensure it's installed via `npm install -g pnpm` if needed.

---

## Security Considerations

### Access Control

- Keep admin credentials secure and rotate them regularly
- Use environment variables for sensitive configuration
- Never commit `.env` files containing credentials
- Restrict database ports to localhost when possible

### Network Security

- Configure firewall rules to restrict database access
- Use TLS/SSL for external connections
- Monitor access logs for suspicious activity
- Keep Docker and database software updated

### Data Protection

- Implement regular backup procedures
- Encrypt sensitive data at rest
- Follow the principle of least privilege
- Document all security configurations

---

## Database Access

### Connection Endpoints

- **Database API**: [http://localhost:3210](http://localhost:3210)
- **Admin Dashboard**: [http://localhost:6791](http://localhost:6791)

### Database Administration

The Convex Admin Dashboard at [http://localhost:6791](http://localhost:6791) provides a web interface for:

- Monitoring database health
- Managing data and schemas
- Viewing logs and metrics
- Running queries

### Admin Authentication

To access the admin dashboard:

1. **Quick Setup (Recommended):**

   ```bash
   pnpm run self-hosted:setup
   ```

   This will display the deployment URL, dashboard URL, and admin key.

2. **Or generate credentials separately:**

   ```bash
   pnpm run docker:generate-admin-key
   ```

3. **Use the generated key for dashboard login** - Format:

   ```
   instance-name|admin-key-hash
   ```

   Example: `convex-tutorial-local|01f7a735340227d2769630a37069656211287635ea7cc4737e98d517cbda803723deccea7b4b848d1d797975102c2c7328`

4. **Credential Storage**: The admin key is automatically saved in `.devcontainer/.env.local` as `CONVEX_SELF_HOSTED_ADMIN_KEY`

> **Security Note**: The admin key grants full database access. Store it securely and never share it.

---

## Troubleshooting

### Database Connection Issues

If unable to connect to the database:

1. Verify Docker container status: `docker ps`
2. Check database logs: `pnpm run docker:logs`
3. Ensure ports 3210 and 6791 are available
4. Validate connection settings in `.devcontainer/.env.local`

### Admin Dashboard Access

If dashboard authentication fails:

1. Generate new credentials: `pnpm run docker:generate-admin-key`
2. Verify admin key in `.devcontainer/.env.local`
3. Use complete key format: `instance-name|admin-key-hash`

### Schema Deployment Issues

If schema changes aren't reflecting:

1. Execute `pnpm run deploy-functions`
2. Check deployment logs for errors
3. Verify schema file syntax

### Data Persistence

If data isn't persisting between restarts:

1. Check Docker volume configuration
2. Verify database shutdown was clean
3. Review backup settings if configured

### Database Management

```bash
# Monitor database status
docker ps --filter "name=convex"

# View detailed database logs
docker compose logs convex-backend

# Inspect database volumes
docker volume ls --filter "name=convex"
```

### Data Backup and Recovery

```bash
# Create full database backup
docker run --rm --volumes-from convex-backend -v $(pwd):/backup alpine tar cvf /backup/convex-data.tar /data

# Export database schema
pnpm run deploy-functions -- --dry-run > schema-backup.json

# Restore from backup
docker run --rm --volumes-from convex-backend -v $(pwd):/backup alpine tar xvf /backup/convex-data.tar

# Reset database (⚠️ Warning: Deletes all data!)
docker compose down -v && docker volume prune -f
```

### Maintenance

```bash
# Rebuild database container
docker compose build convex-backend

# Check database health
curl http://localhost:3210/_system/health

# View database metrics
curl http://localhost:3210/_system/metrics
```

---

## Security Notes

- **Always change the `INSTANCE_SECRET` in `.env.docker` before deploying to any non-local environment!**
- **Keep your admin key secure - it provides full access to your Convex backend**
- **Never commit environment files containing secrets to version control**

---

#### Port Configuration

The following ports are automatically forwarded:

- **5173**: Vite development server (your React app)
- **3210**: Convex backend server
- **6791**: Convex dashboard

#### Accessing Your App

Once the development server is running:

1. Click on the "Ports" tab in VS Code
2. Click the globe icon next to port 5173 to open your app
3. Access the Convex dashboard on port 6791 (use the admin key as password)

## Generated Files Explained

The files in `convex/_generated/` are **automatically created** by Convex and are **essential** for your application:

### 🔧 **api.js & api.d.ts**

- **Purpose**: Provide typed API references for your frontend
- **Contains**: `api.example.listItems`, `api.example.addItem`, `api.example.deleteItem` references
- **Used by**: Your frontend app imports these to call your backend functions
- **Auto-regenerated**: Every time you run `convex dev` or `deploy-functions`

### 🔧 **server.js & server.d.ts**

- **Purpose**: Provide server-side utilities (`mutation`, `query`, `action`)
- **Contains**: TypeScript definitions for Convex function builders
- **Used by**: `convex/example.ts` imports `mutation` and `query` from here
- **Auto-regenerated**: When Convex analyzes your schema and functions

### 🔧 **dataModel.d.ts**

- **Purpose**: TypeScript definitions for your database schema
- **Contains**: Table definitions, document types, and ID types
- **Note**: Currently permissive (`Doc = any`) because no schema.ts exists
- **Auto-regenerated**: When you add a `convex/schema.ts` file

### 📋 **Essential Files**

- **`convex/example.ts`** - Example functions (customize for your needs)
- **`ADMIN_KEY_WORKFLOW.md`** - Admin key management documentation
- **`docker-build/`** - Docker build scripts and utilities
- **`docker-compose.yml`** - Container orchestration
- **All files in `convex/_generated/`** - Auto-generated API bindings

## Customizing Your Functions

Replace the example functions in `convex/example.ts` with your own:

```typescript
// convex/yourFunctions.ts
import { mutation, query } from "./_generated/server";
import { v } from "convex/values";

export const yourQuery = query({
  args: {
    /* your args */
  },
  handler: async (ctx, args) => {
    // Your query logic
  },
});

export const yourMutation = mutation({
  args: {
    /* your args */
  },
  handler: async (ctx, args) => {
    // Your mutation logic
  },
});
```

After adding functions, run `pnpm run deploy-functions` to update the generated API.

## Intended Use Cases

### Perfect For:

**🚀 Rapid Prototyping**

- Skip API development entirely - write database functions directly
- Real-time features out of the box (live updates, collaborative editing)
- Type-safe from database to frontend without manual API contracts

**📱 Modern Web Applications**

- **Chat applications**: Real-time messaging with automatic subscriptions
- **Collaborative tools**: Live document editing, shared whiteboards
- **Dashboards**: Real-time analytics and monitoring interfaces
- **Social features**: Live comments, reactions, notifications

**🏢 Enterprise Applications**

- **Admin panels**: CRUD operations with real-time updates
- **Workflow management**: Task tracking with live status updates
- **Team collaboration**: Project management with real-time sync
- **Customer support**: Live chat and ticket management

**🔧 Developer Experience**

- **No API boilerplate**: Functions are your API endpoints
- **Automatic optimizations**: Query batching, caching, subscriptions
- **Built-in auth**: User management and permissions
- **File storage**: Handle uploads and file management
- **Full-text search**: Built-in search capabilities

### What You Get vs Traditional Stack:

| Traditional Stack               | Convex BaaS                  |
| ------------------------------- | ---------------------------- |
| Write API routes manually       | Functions auto-generate API  |
| Set up WebSockets for real-time | Real-time by default         |
| Manage database migrations      | Schema evolution built-in    |
| Handle caching manually         | Automatic query optimization |
| Write separate validation       | Type-safe end-to-end         |
| Set up auth from scratch        | Built-in authentication      |
| Configure file uploads          | Integrated file storage      |

### When NOT to Use:

- **Heavy computational workloads**: Use dedicated compute services
- **Complex SQL requirements**: Stick with PostgreSQL/MySQL
- **Existing large codebases**: Migration might be complex
- **Specific database features**: If you need Redis, Elasticsearch, etc.

## Files Removed (Boilerplate Cleanup)

```

```
