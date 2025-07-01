# Admin Key Management Workflow

This document explains the admin key management system for your Convex self-hosted setup.

## Quick Start

### 1. Start Convex and Generate Admin Key

```bash
pnpm run self-hosted:setup
```

This command will:

- Start the Docker containers
- Wait for the backend to be ready
- Generate and display the admin key in the console

### 2. Access the Convex Dashboard

To access your self-hosted Convex dashboard:

1. **Deployment URL**: Use `http://localhost:3210` (the port where your Convex backend is running)
2. **Admin Key**: Copy the admin key from the console output after running the setup command

### 3. Generate a New Admin Key

If you need a new admin key while containers are running:

```bash
pnpm run docker:generate-admin-key
```

## Available Scripts

### Setup & Management

- `pnpm run self-hosted:setup` - Start containers and generate admin key
- `pnpm run self-hosted:stop` - Stop all containers

### Docker Operations

- `pnpm run docker:up` - Start containers only
- `pnpm run docker:down` - Stop containers
- `pnpm run docker:logs` - View container logs
- `pnpm run docker:generate-admin-key` - Generate admin key (containers must be running)

### Cleanup & Reset

- `pnpm run docker:reset-images` - Stop containers and remove all Docker images/volumes
- `pnpm run docker:reset-full` - Complete cleanup and Docker reset

## Admin Key Usage

The admin key is displayed in your console when you run the setup command. Example output:

```
Admin key:
convex-tutorial-local|013891c8927ad5db77b2928862ee56456ccd83e4c6d06d9f6873492bb68a2b01b3fb4c8926bcfe88cf8f536497f55fd87e
```

**Important**: Copy this key immediately as it's only displayed in the console.

## Workflow Benefits

1. **Automated**: No manual steps to generate admin keys
2. **Simple**: Admin keys are displayed directly in console
3. **On-demand**: Generate new keys anytime with a single command
4. **Clean Reset**: Easy cleanup commands for development cycles

## Development Cycle

```bash
# Start fresh
pnpm run self-hosted:setup

# Work with your application...

# Reset everything when needed
pnpm run docker:reset-full

# Start again
pnpm run self-hosted:setup
```
