# Convex Backend for Telegram Bot

This directory contains the Convex backend functions and schema for a Telegram bot application. The backend provides real-time data synchronization, message storage, and API endpoints for both the Telegram bot and web dashboard.

## Architecture Overview

The system is built around two main data entities:

- **Messages**: Individual Telegram messages with metadata
- **Threads**: Conversation threads that group related messages

## File Structure

### Core Schema

- **`schema.ts`** - Defines the database schema with tables for messages and threads
- **`tsconfig.json`** - TypeScript configuration for Convex functions

### Data Access Layer

- **`messages.ts`** - Mutations and queries for message operations
- **`threads.ts`** - Mutations and queries for thread management

### API Layer

- **`http.ts`** - HTTP router configuration and endpoint definitions
- **`api.ts`** - HTTP action handlers for external API access
- **`telegram.ts`** - Duplicate API handlers (legacy, consider consolidating)

## Detailed File Descriptions

### `schema.ts`

Defines two main tables:

- `telegram_messages`: Stores individual messages with user info, content, and thread references
- `telegram_threads`: Manages conversation threads with metadata and message counts

Key indexes support efficient queries by chat, user, thread, and timestamp.

### `messages.ts`

Provides:

- `saveMessage`: Mutation to store incoming messages and auto-create/update user threads
- `getMessagesByChatId`: Query messages by chat ID
- `getAllMessages`: Query all messages with pagination
- `getMessagesByThread`: Query messages within a specific thread
- `getMessagesByThreadDoc`: Query messages using thread document ID

### `threads.ts`

Provides:

- `upsertThread`: Create or update thread information
- `getThreadsInChat`: Get all threads in a specific chat
- `getAllActiveThreads`: Get active threads across all chats
- `getThread`: Get specific thread by chat and thread ID
- `getThreadById`: Get thread by document ID

### `http.ts`

HTTP router that exposes:

- `POST /api/telegram/messages`: Save incoming messages from Telegram bot
- `GET /api/telegram/messages`: Retrieve messages with optional filtering
- `GET /api/health`: Health check endpoint

**Note**: Message sending is now handled directly by the Next.js app using `node-telegram-bot-api` <mcreference link="https://www.npmjs.com/package/node-telegram-bot-api" index="0">0</mcreference>

### `api.ts`

HTTP action handlers:

- `saveMessageAPI`: Processes incoming message data from Telegram webhook
- `getMessagesAPI`: Retrieves messages with optional chat ID and limit filters

### `telegram.ts`

⚠️ **Note**: This file duplicates functionality from `api.ts`. Consider consolidating to avoid code duplication.

## Data Flow

### Incoming Messages (Telegram → Database)

1. Telegram bot receives message
2. Bot calls `POST /api/telegram/messages`
3. `saveMessageAPI` validates and processes the message
4. `saveMessage` mutation stores message and manages thread state
5. Thread is auto-created or updated based on user ID

### Outgoing Messages (Next.js App → Telegram)

1. Next.js app uses `node-telegram-bot-api` to send message directly to Telegram <mcreference link="https://www.npmjs.com/package/node-telegram-bot-api" index="0">0</mcreference>
2. After successful send, Next.js app calls `POST /api/telegram/messages` to save message
3. `saveMessageAPI` stores the sent message in the database
4. Thread state is automatically updated

### Message Retrieval

1. Web dashboard queries messages via Convex queries
2. Real-time updates are pushed to connected clients
3. Messages can be filtered by chat, thread, or user

## Environment Variables

- `CONVEX_URL`: Convex deployment URL (configured in deployment)
- `TELEGRAM_BOT_TOKEN`: Required in Next.js app for sending messages via `node-telegram-bot-api` <mcreference link="https://www.npmjs.com/package/node-telegram-bot-api" index="0">0</mcreference>

## API Endpoints

| Method | Endpoint                 | Description                               |
| ------ | ------------------------ | ----------------------------------------- |
| POST   | `/api/telegram/messages` | Save incoming Telegram messages           |
| GET    | `/api/telegram/messages` | Retrieve messages (with optional filters) |
| GET    | `/api/health`            | Health check                              |

## Thread Management

The system automatically creates conversation threads based on:

- **User-based threads**: One thread per user per chat
- **Message thread ID**: Telegram's native thread support
- **Thread metadata**: Includes creator info, message counts, and timestamps

## Real-time Features

Convex provides real-time synchronization:

- Live message updates across all connected clients
- Automatic thread state updates
- Real-time message counts and timestamps

## Development Notes

### Code Quality Improvements

1. **Consolidate duplicate code**: Merge `api.ts` and `telegram.ts`
2. **Add input validation**: Implement comprehensive request validation
3. **Error handling**: Add structured error responses
4. **Type safety**: Ensure all API responses are properly typed
5. **Documentation**: Add JSDoc comments to all functions

### Performance Considerations

1. **Pagination**: All queries include limit parameters
2. **Indexing**: Database indexes optimize common query patterns
3. **Caching**: Consider adding caching for frequently accessed data

### Security

1. **Environment variables**: Sensitive tokens are stored securely
2. **Input validation**: All API endpoints validate required fields
3. **Error messages**: Avoid exposing internal details in error responses

## Usage Examples

### Querying Messages

```typescript
// Get recent messages in a chat
const messages = await ctx.runQuery(api.messages.getMessagesByChatId, {
  chatId: 12345,
  limit: 50,
});

// Get messages in a specific thread
const threadMessages = await ctx.runQuery(api.messages.getMessagesByThread, {
  chatId: 12345,
  messageThreadId: 67890,
  limit: 20,
});
```

### Sending Messages (Next.js App)

```typescript
// In your Next.js API route
import TelegramBot from "node-telegram-bot-api";

const bot = new TelegramBot(process.env.TELEGRAM_BOT_TOKEN!);

// Send message to Telegram
const telegramResult = await bot.sendMessage(chatId, text, {
  message_thread_id: messageThreadId, // optional
});

// Then save to Convex database
const saveResult = await fetch("/api/telegram/messages", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    messageId: telegramResult.message_id,
    chatId: chatId,
    text: text,
    messageType: "bot_message",
    timestamp: telegramResult.date * 1000,
    messageThreadId: messageThreadId,
  }),
});
```

### Managing Threads

```typescript
// Get all threads in a chat
const threads = await ctx.runQuery(api.threads.getThreadsInChat, {
  chatId: 12345,
  limit: 10,
});

// Create or update a thread
const threadId = await ctx.runMutation(api.threads.upsertThread, {
  threadId: 67890,
  chatId: 12345,
  title: "Support Discussion",
  creatorUserId: 11111,
});
```

This backend provides a robust foundation for a Telegram bot with real-time web dashboard capabilities, supporting both individual messages and threaded conversations.
