"use client";

import { useQuery } from "convex/react";
import { api } from "../../../convex/_generated/api";
import { Id } from "../../../convex/_generated/dataModel";
import { TelegramMessage } from "../../models/telegram";
import { Hero } from "../../components/ui/hero";
import { Card } from "../../components/ui/card";
import Link from "next/link";
import {
  ArrowLeft,
  Send,
  Bot,
  Clock,
  MessageSquare,
  User,
  Hash,
} from "lucide-react";
import { cn } from "../../lib/utils";
import React, { useState } from "react";
import { Button } from "../../components/ui/button";

interface ThreadDetailPageProps {
  params: Promise<{ threadId: string }>;
}

export default function ThreadDetailPage({ params }: ThreadDetailPageProps) {
  const [threadId, setThreadId] = useState<string>("");
  const [newMessage, setNewMessage] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Unwrap params
  React.useEffect(() => {
    params.then((p) => setThreadId(p.threadId));
  }, [params]);

  const messages = useQuery(
    api.messages.getMessagesByThreadDoc,
    threadId ? { threadDocId: threadId as Id<"telegram_threads"> } : "skip"
  );

  // Get thread info directly by document ID
  const thread = useQuery(
    api.threads.getThreadById,
    threadId ? { threadDocId: threadId as Id<"telegram_threads"> } : "skip"
  );

  const handleSendMessage = async (e: React.FormEvent) => {
    e.preventDefault();
    console.log("=== FRONTEND: Send message clicked ===");
    console.log("New message:", newMessage);
    console.log("Thread:", thread);

    if (!newMessage.trim() || !thread) {
      console.log("❌ FRONTEND: Validation failed - missing message or thread");
      return;
    }

    setIsLoading(true);
    setError(null);

    console.log("✅ FRONTEND: Starting message send process");

    try {
      const payload = {
        chatId: thread.chatId,
        text: newMessage,
        threadDocId: threadId, // Pass the thread document ID
        messageThreadId: thread.threadId, // Pass the Telegram thread ID
      };

      console.log("📤 FRONTEND: Sending request to thread-specific API");
      console.log("Payload:", JSON.stringify(payload, null, 2));

      const response = await fetch("/api/telegram/send-to-thread", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify(payload),
      });

      console.log("📥 FRONTEND: Received response from API");
      console.log("Response status:", response.status);
      console.log("Response ok:", response.ok);

      const result = await response.json();
      console.log("Response body:", JSON.stringify(result, null, 2));

      if (!response.ok) {
        console.log("❌ FRONTEND: API returned error");
        throw new Error(result.error || "Failed to send message");
      }

      console.log("✅ FRONTEND: Message sent successfully");
      setNewMessage("");
    } catch (err: unknown) {
      console.error("❌ FRONTEND: Error in handleSendMessage:");
      console.error("Error type:", (err as Error)?.constructor?.name);
      console.error("Error message:", (err as Error)?.message);
      console.error("Full error:", err);
      setError(err instanceof Error ? err.message : "Failed to send message");
    } finally {
      setIsLoading(false);
      console.log("🏁 FRONTEND: Send message process completed");
    }
  };

  if (!threadId) {
    return (
      <div className="max-w-4xl mx-auto p-6">
        <div className="animate-pulse text-center text-gray-500 dark:text-gray-400">
          Loading...
        </div>
      </div>
    );
  }

  if (messages === undefined) {
    return (
      <div className="max-w-4xl mx-auto p-6">
        <div className="animate-pulse text-center text-gray-500 dark:text-gray-400">
          Loading thread messages...
        </div>
      </div>
    );
  }

  if (messages.length === 0) {
    return (
      <div className="relative min-h-screen">
        <div className="relative z-20 min-h-screen flex flex-col items-center justify-center px-4 pt-24 pb-20">
          <div className="max-w-4xl w-full mx-auto">
            <div className="mb-6">
              <Link
                href="/threads"
                className="inline-flex items-center text-blue-600 dark:text-blue-400 hover:text-blue-800 dark:hover:text-blue-300 font-medium transition-colors gap-2"
              >
                <ArrowLeft className="w-4 h-4" />
                Back to Threads
              </Link>
            </div>
            <Hero title="Thread Not Found" whiteText />
            <Card className="bg-gray-900/90 border-gray-700/50">
              <p className="text-center text-gray-500 dark:text-gray-400">
                No messages found for this thread.
              </p>
            </Card>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="relative min-h-screen">
      <div className="relative z-20 min-h-screen flex flex-col items-center justify-center px-4 pt-24 pb-20">
        <div className="max-w-4xl w-full mx-auto">
          <div className="mb-6">
            <Link
              href="/threads"
              className="inline-flex items-center text-blue-600 dark:text-blue-400 hover:text-blue-800 dark:hover:text-blue-300 font-medium transition-colors gap-2"
            >
              <ArrowLeft className="w-4 h-4" />
              Back to Threads
            </Link>
          </div>

          <Hero
            title={thread?.title || `Thread ${thread?.threadId || "Unknown"}`}
            subtitle={`Chat: ${messages[0].chatId} • ${
              messages.length
            } messages • ${thread?.isActive ? "Active" : "Inactive"}`}
            whiteText
          />

          <div className="flex flex-wrap items-center gap-4 text-sm mb-6">
            <span className="inline-flex items-center gap-2 px-3 py-1 bg-blue-100 dark:bg-blue-900 text-blue-800 dark:text-blue-200 rounded-full font-medium">
              <Hash className="w-4 h-4" />
              {messages[0].chatId}
            </span>
            <span className="inline-flex items-center gap-2 px-3 py-1 bg-gray-100 dark:bg-gray-700 text-gray-800 dark:text-gray-200 rounded-full font-medium">
              <MessageSquare className="w-4 h-4" />
              {messages.length} messages
            </span>
            {thread && (
              <span
                className={cn(
                  "inline-flex items-center gap-2 px-3 py-1 rounded-full font-medium",
                  thread.isActive
                    ? "bg-green-100 dark:bg-green-900 text-green-800 dark:text-green-200"
                    : "bg-red-100 dark:bg-red-900 text-red-800 dark:text-red-200"
                )}
              >
                <div
                  className={cn(
                    "w-2 h-2 rounded-full",
                    thread.isActive ? "bg-green-500" : "bg-red-500"
                  )}
                />
                {thread.isActive ? "Active" : "Inactive"}
              </span>
            )}
          </div>

          <div className="space-y-4">
            {messages?.map((message: TelegramMessage) => (
              <Card
                key={message._id}
                className="bg-gray-900/90 border-gray-700/50 hover:shadow-lg transition-shadow"
              >
                <div className="flex justify-between items-start mb-3">
                  <div className="flex items-center gap-3">
                    <div className="flex items-center gap-2">
                      {message.messageType === "bot_message" ? (
                        <Bot className="w-5 h-5 text-purple-400" />
                      ) : (
                        <User className="w-5 h-5 text-blue-400" />
                      )}
                      {message.firstName && (
                        <span className="font-semibold text-gray-200">
                          {message.firstName} {message.lastName}
                        </span>
                      )}
                    </div>
                    {message.username && (
                      <span className="text-blue-400 font-medium">
                        @{message.username}
                      </span>
                    )}
                    {message.messageType === "bot_message" && (
                      <span className="inline-flex items-center gap-2 px-2 py-1 bg-purple-900/50 text-purple-200 text-xs font-medium rounded-full">
                        <Bot className="w-3 h-3" />
                        Bot
                      </span>
                    )}
                  </div>
                  <div className="inline-flex items-center gap-2 text-sm text-gray-400">
                    <Clock className="w-4 h-4" />
                    {new Date(message.timestamp).toLocaleString()}
                  </div>
                </div>
                <div className="mb-3">
                  <p className="text-gray-200 leading-relaxed">
                    {message.text}
                  </p>
                </div>
                <div className="flex justify-between items-center text-xs text-gray-400">
                  <span className="inline-flex items-center gap-2">
                    <Hash className="w-3 h-3" />
                    {message.messageId}
                  </span>
                  <span className="inline-flex items-center gap-2 px-2 py-1 bg-gray-800 rounded-full">
                    <MessageSquare className="w-3 h-3" />
                    {message.messageType}
                  </span>
                </div>
              </Card>
            ))}
          </div>

          {thread && thread.isActive && (
            <Card className="mt-6 bg-gray-900/90 border-gray-700/50">
              <h3 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
                <Send className="w-5 h-5 text-blue-400" />
                Send Message to Thread
              </h3>
              <form onSubmit={handleSendMessage} className="space-y-4">
                <div>
                  <textarea
                    value={newMessage}
                    onChange={(e) => setNewMessage(e.target.value)}
                    placeholder="Type your message here..."
                    className="w-full px-4 py-3 border-2 border-gray-700 rounded-xl focus:border-blue-500 focus:outline-none transition-colors bg-gray-800 text-white resize-vertical placeholder-gray-400"
                    rows={3}
                    disabled={isLoading}
                  />
                </div>
                {error && (
                  <div className="p-4 rounded-xl font-medium bg-red-900/50 text-red-200 border border-red-700/50 flex items-center gap-2">
                    <div className="w-2 h-2 rounded-full bg-red-500" />
                    {error}
                  </div>
                )}
                <Button
                  onClick={handleSendMessage}
                  disabled={isLoading || !newMessage.trim()}
                  className="w-full bg-blue-600 hover:bg-blue-700 hover:ring-blue-500"
                >
                  Send Message
                </Button>
              </form>
            </Card>
          )}
        </div>
      </div>
    </div>
  );
}
