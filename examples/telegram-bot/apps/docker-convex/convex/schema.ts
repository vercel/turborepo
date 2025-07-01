// apps/docker-convex/convex/schema.ts
import { defineSchema, defineTable } from "convex/server";
import { v } from "convex/values";

export default defineSchema({
  // Telegram threads table - represents conversation threads
  telegram_threads: defineTable({
    threadId: v.number(), // Telegram's message_thread_id
    chatId: v.number(), // Chat where this thread exists
    title: v.optional(v.string()), // Thread title if available
    creatorUserId: v.optional(v.number()), // User who created the thread
    creatorUsername: v.optional(v.string()),
    creatorFirstName: v.optional(v.string()),
    creatorLastName: v.optional(v.string()),
    firstMessageId: v.optional(v.number()), // ID of the first message in thread
    lastMessageId: v.optional(v.number()), // ID of the most recent message
    lastMessageText: v.optional(v.string()), // Preview of last message
    lastMessageTimestamp: v.optional(v.number()), // Timestamp of last message
    messageCount: v.number(), // Total number of messages in thread
    isActive: v.boolean(), // Whether thread is still active
    createdAt: v.number(), // When thread was first seen
    updatedAt: v.number(), // When thread was last updated
  })
    .index("by_chat_id", ["chatId"])
    .index("by_thread_id", ["threadId"])
    .index("by_chat_and_thread", ["chatId", "threadId"])
    .index("by_chat_and_user", ["chatId", "creatorUserId"])
    .index("by_active", ["isActive"])
    .index("by_last_message", ["lastMessageTimestamp"])
    .index("by_active_with_timestamp", ["isActive", "lastMessageTimestamp"]),

  // Telegram messages table
  telegram_messages: defineTable({
    messageId: v.number(),
    chatId: v.number(),
    userId: v.optional(v.number()),
    username: v.optional(v.string()),
    firstName: v.optional(v.string()),
    lastName: v.optional(v.string()),
    text: v.string(),
    messageType: v.string(), // "text", "photo", "document", etc.
    timestamp: v.number(), // Unix timestamp from Telegram
    createdAt: v.number(), // When the record was created in our DB
    // Thread support
    messageThreadId: v.optional(v.number()), // Telegram thread ID if message is in a thread
    replyToMessageId: v.optional(v.number()), // ID of message this is replying to
    // Reference to our thread record
    threadDocId: v.optional(v.id("telegram_threads")), // Reference to telegram_threads table
  })
    .index("by_chat_id", ["chatId"])
    .index("by_user_id", ["userId"])
    .index("by_timestamp", ["timestamp"])
    .index("by_thread", ["chatId", "messageThreadId"])
    .index("by_thread_doc", ["threadDocId"])
    .index("by_reply", ["replyToMessageId"]),
});
