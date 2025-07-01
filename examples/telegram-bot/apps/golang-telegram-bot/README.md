# 📦 Go Telegram Bot

A Telegram bot built with Go that echoes messages back to users. The bot responds to any text message with "message received: [your message]" and includes comprehensive logging for monitoring.

## ✨ Features

- 📨 Responds to all text messages with detailed logging
- 🐳 Fully Dockerized for easy deployment
- ⚙️ Environment variable configuration
- 🛑 Graceful shutdown handling
- 📋 Comprehensive logging with timestamps and user information
- 🔧 Package.json-like development interface via Makefile
- 🎨 Color-coded terminal output for better UX

## 🚀 Prerequisites

- Docker installed on your system
- Go 1.24+ (for local development)
- A Telegram bot token from [@BotFather](https://t.me/botfather)

## 📋 Quick Start

### 1. Get a Telegram Bot Token

1. Open Telegram and search for [@BotFather](https://t.me/botfather)
2. Start a chat and send `/newbot`
3. Follow the instructions to create your bot
4. Copy the bot token provided by BotFather

### 2. Setup and Run

```bash
# Install dependencies and setup environment
make install

# Setup environment file (creates .env from .env.example)
make env-setup

# Edit .env with your bot token
# TELEGRAM_TOKEN=your_actual_bot_token_here

# Start the bot in Docker
make start
```

## 🔧 Development Commands

This project uses a comprehensive Makefile that provides a package.json-like interface for development:

### 🚀 Quick Start Commands

| Command        | Description                                |
| -------------- | ------------------------------------------ |
| `make install` | Install dependencies and setup environment |
| `make dev`     | Run in development mode (local Go)         |
| `make start`   | Build and run in Docker                    |

### 🔧 Development Workflow

| Command        | Description                              |
| -------------- | ---------------------------------------- |
| `make build`   | Build Docker image                       |
| `make run`     | Run container with environment variables |
| `make stop`    | Stop running container                   |
| `make restart` | Restart the application                  |
| `make logs`    | View container logs in real-time         |
| `make shell`   | Open shell in running container          |

### 🧹 Cleanup Commands

| Command             | Description                      |
| ------------------- | -------------------------------- |
| `make clean`        | Stop and remove app containers   |
| `make clean-all`    | Remove all containers and images |
| `make docker-prune` | Clean up Docker system           |

### 🔍 Quality & Testing

| Command       | Description                          |
| ------------- | ------------------------------------ |
| `make test`   | Run Go tests                         |
| `make lint`   | Run linter (golangci-lint or go vet) |
| `make format` | Format code and tidy modules         |
| `make deps`   | Update dependencies                  |

### ⚙️ Utilities

| Command          | Description                      |
| ---------------- | -------------------------------- |
| `make status`    | Show comprehensive system status |
| `make env-check` | Verify environment setup         |
| `make env-setup` | Setup environment files          |
| `make git-setup` | Setup git with main branch       |

### 🔗 Composite Commands

For common workflows, you can chain commands together:

```bash
# Complete rebuild and restart
make clean && make build && make start

# Clean everything and fresh start
make clean-all && make docker-prune && make start

# Development cycle: format, test, and restart
make format && make test && make restart

# Full cleanup and rebuild
make stop && make clean && make docker-prune && make build && make start
```

### Manual Docker Commands

If you prefer to use Docker directly:

```bash
# Build the image
docker build -t my-bot .

# Run the container
docker run --env-file .env my-bot
```

## How It Works

1. The bot reads the `TELEGRAM_TOKEN` from the environment variables
2. It connects to Telegram's Bot API using long polling
3. When a user sends a text message, the bot responds with "message received: [original message]"
4. The bot handles graceful shutdown when interrupted (Ctrl+C)

## Project Structure

```
.
├── .env                 # Environment variables (not in git)
├── .env.example         # Environment variables template
├── .gitignore          # Git ignore file
├── .dockerignore       # Docker ignore file
├── Dockerfile          # Docker configuration
├── Makefile           # Build and run commands
├── README.md          # This file
├── go.mod             # Go module definition
├── go.sum             # Go module checksums
└── main.go            # Main application code
```

## Development

### Running Locally (without Docker)

1. Install Go 1.24 or later
2. Set up your environment variables:
   ```bash
   export TELEGRAM_TOKEN=your_bot_token_here
   ```
3. Install dependencies:
   ```bash
   go mod tidy
   ```
4. Run the bot:
   ```bash
   go run main.go
   ```

### Customizing the Bot

To modify the bot's behavior, edit the `messageHandler` function in `main.go`. You can:

- Change the response message format
- Add command handling (e.g., `/start`, `/help`)
- Add support for different message types (photos, documents, etc.)
- Implement more complex logic

## Troubleshooting

### Bot Not Responding

1. Check that your bot token is correct in the `.env` file
2. Ensure the bot is running (check Docker logs: `docker logs <container_id>`)
3. Make sure you've started a chat with your bot on Telegram
4. Verify that your bot token has the necessary permissions

### Docker Issues

1. Make sure Docker is running
2. Check that the `.env` file exists and contains your bot token
3. Try rebuilding the image: `make clean-docker && make build`

## License

This project is open source and available under the [MIT License](LICENSE).

## Contributing

Feel free to submit issues and enhancement requests!
