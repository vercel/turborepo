import { mutation, query } from "./_generated/server";
import { v } from "convex/values";

// Mutation to create or update a telegram thread
export const upsertThread = mutation({
  args: {
    threadId: v.number(),
    chatId: v.number(),
    title: v.optional(v.string()),
    creatorUserId: v.optional(v.number()),
    creatorUsername: v.optional(v.string()),
    creatorFirstName: v.optional(v.string()),
    creatorLastName: v.optional(v.string()),
    firstMessageId: v.optional(v.number()),
    lastMessageId: v.optional(v.number()),
    lastMessageText: v.optional(v.string()),
    lastMessageTimestamp: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    // Check if thread already exists
    const existingThread = await ctx.db
      .query("telegram_threads")
      .withIndex("by_chat_and_thread", (q) =>
        q.eq("chatId", args.chatId).eq("threadId", args.threadId)
      )
      .first();

    if (existingThread) {
      // Update existing thread
      await ctx.db.patch(existingThread._id, {
        title: args.title || existingThread.title,
        lastMessageId: args.lastMessageId || existingThread.lastMessageId,
        lastMessageText: args.lastMessageText || existingThread.lastMessageText,
        lastMessageTimestamp:
          args.lastMessageTimestamp || existingThread.lastMessageTimestamp,
        messageCount: existingThread.messageCount + 1,
        updatedAt: Date.now(),
      });
      return existingThread._id;
    } else {
      // Create new thread
      const threadDocId = await ctx.db.insert("telegram_threads", {
        threadId: args.threadId,
        chatId: args.chatId,
        title: args.title,
        creatorUserId: args.creatorUserId,
        creatorUsername: args.creatorUsername,
        creatorFirstName: args.creatorFirstName,
        creatorLastName: args.creatorLastName,
        firstMessageId: args.firstMessageId,
        lastMessageId: args.lastMessageId,
        lastMessageText: args.lastMessageText,
        lastMessageTimestamp: args.lastMessageTimestamp,
        messageCount: 1,
        isActive: true,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      });
      return threadDocId;
    }
  },
});

// Query to get all threads in a chat (using the new telegram_threads table)
export const getThreadsInChat = query({
  args: {
    chatId: v.number(),
    limit: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    const limit = args.limit || 50;
    return await ctx.db
      .query("telegram_threads")
      .withIndex("by_chat_id", (q) => q.eq("chatId", args.chatId))
      .order("desc")
      .take(limit);
  },
});

// Query to get all active threads across all chats
export const getAllActiveThreads = query({
  args: {
    limit: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    const limit = args.limit || 50;
    return await ctx.db
      .query("telegram_threads")
      .withIndex("by_active", (q) => q.eq("isActive", true))
      .order("desc")
      .take(limit);
  },
});

// Query to get a specific thread by threadId and chatId
export const getThread = query({
  args: {
    chatId: v.number(),
    threadId: v.number(),
  },
  handler: async (ctx, args) => {
    return await ctx.db
      .query("telegram_threads")
      .withIndex("by_chat_and_thread", (q) =>
        q.eq("chatId", args.chatId).eq("threadId", args.threadId)
      )
      .first();
  },
});

// Query to get a specific thread by document ID
export const getThreadById = query({
  args: {
    threadDocId: v.id("telegram_threads"),
  },
  handler: async (ctx, args) => {
    return await ctx.db.get(args.threadDocId);
  },
});
