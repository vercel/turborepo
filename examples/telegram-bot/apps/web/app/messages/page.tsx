// apps/web/app/messages/page.tsx
"use client";

import { useQuery } from "convex/react";
import { api } from "../../convex/_generated/api";
import { Hero } from "../components/ui/hero";
import { Card } from "../components/ui/card";

interface TelegramMessage {
  _id: string;
  messageId: number;
  chatId: number;
  userId?: number;
  username?: string;
  firstName?: string;
  lastName?: string;
  text: string;
  messageType: string;
  timestamp: number;
  createdAt: number;
  messageThreadId?: number;
  replyToMessageId?: number;
}

export default function MessagesPage() {
  const messages = useQuery(api.messages.getAllMessages, { limit: 100 });

  if (messages === undefined) {
    return (
      <div className="max-w-6xl mx-auto p-6">
        <div className="text-center py-16">
          <div className="text-xl text-gray-600 dark:text-gray-300">
            Loading messages...
          </div>
        </div>
      </div>
    );
  }

  const formatDate = (timestamp: number) => {
    return new Date(timestamp).toLocaleString();
  };

  const getUserDisplay = (message: TelegramMessage) => {
    if (message.username) return `@${message.username}`;
    if (message.firstName || message.lastName) {
      return `${message.firstName || ""} ${message.lastName || ""}`.trim();
    }
    return `User ${message.userId || "Unknown"}`;
  };

  // Group messages by threads
  const groupMessagesByThread = (messages: TelegramMessage[]) => {
    const threads: { [key: string]: TelegramMessage[] } = {};
    const standaloneMessages: TelegramMessage[] = [];

    messages.forEach((message) => {
      if (message.messageThreadId) {
        const threadKey = `${message.chatId}-${message.messageThreadId}`;
        if (!threads[threadKey]) {
          threads[threadKey] = [];
        }
        threads[threadKey].push(message);
      } else {
        standaloneMessages.push(message);
      }
    });

    // Sort messages within each thread by timestamp
    Object.keys(threads).forEach((threadKey) => {
      const threadMessages = threads[threadKey];
      if (threadMessages) {
        threadMessages.sort((a, b) => a.timestamp - b.timestamp);
      }
    });

    return { threads, standaloneMessages };
  };

  const { threads, standaloneMessages } = groupMessagesByThread(messages || []);

  return (
    <div className="max-w-6xl mx-auto p-6">
      <Hero
        title="Telegram Messages"
        subtitle={`Total messages: ${messages.length} | Threads: ${
          Object.keys(threads).length
        } | Standalone: ${standaloneMessages.length}`}
      />

      <div className="flex flex-col gap-6">
        {messages.length === 0 ? (
          <Card className="text-center py-12">
            <p className="text-gray-600 dark:text-gray-400 mb-2">
              No messages found.
            </p>
            <p className="text-gray-500 dark:text-gray-500">
              Send a message to your Telegram bot to see it here!
            </p>
          </Card>
        ) : (
          <>
            {/* Render Threads */}
            {Object.entries(threads).map(([threadKey, threadMessages]) => {
              const firstMessage = threadMessages[0];
              if (!firstMessage) return null;

              return (
                <Card
                  key={threadKey}
                  className="bg-gray-50 dark:bg-gray-800 border-2"
                >
                  <div className="flex justify-between items-center mb-4 pb-3 border-b border-gray-200 dark:border-gray-600">
                    <h3 className="text-lg font-semibold text-gray-700 dark:text-gray-300">
                      Thread {firstMessage.messageThreadId} in Chat{" "}
                      {firstMessage.chatId}
                    </h3>
                    <span className="bg-gray-600 text-white px-3 py-1 rounded-full text-sm font-medium">
                      {threadMessages.length} messages
                    </span>
                  </div>
                  <div className="flex flex-col gap-3">
                    {threadMessages.map((message: TelegramMessage) => (
                      <Card
                        key={message._id}
                        className="ml-4 border-l-4 border-blue-500 bg-white dark:bg-gray-900"
                      >
                        <div className="flex justify-between items-center mb-3 flex-wrap gap-2">
                          <span className="bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200 px-3 py-1 rounded-full text-sm font-medium">
                            {getUserDisplay(message)}
                          </span>
                          <span className="text-gray-500 dark:text-gray-400 text-sm font-mono">
                            {formatDate(message.timestamp)}
                          </span>
                          {message.replyToMessageId && (
                            <span className="bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200 px-2 py-1 rounded-full text-xs italic">
                              ↳ Reply to {message.replyToMessageId}
                            </span>
                          )}
                        </div>
                        <div className="mb-3">
                          <p className="text-gray-900 dark:text-gray-100 leading-relaxed">
                            {message.text}
                          </p>
                        </div>
                        <div className="flex justify-between items-center pt-3 border-t border-gray-100 dark:border-gray-700 text-sm">
                          <span className="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200 px-2 py-1 rounded-full text-xs capitalize">
                            {message.messageType}
                          </span>
                          <span className="text-gray-400 dark:text-gray-500 font-mono text-xs">
                            ID: {message.messageId}
                          </span>
                        </div>
                      </Card>
                    ))}
                  </div>
                </Card>
              );
            })}

            {/* Render Standalone Messages */}
            {standaloneMessages.length > 0 && (
              <div className="mt-8">
                <h3 className="text-xl font-semibold text-gray-700 dark:text-gray-300 mb-4 pb-2 border-b-2 border-gray-200 dark:border-gray-600">
                  Standalone Messages
                </h3>
                {standaloneMessages.map((message: TelegramMessage) => (
                  <Card key={message._id}>
                    <div className="flex justify-between items-center mb-3 flex-wrap gap-2">
                      <span className="bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200 px-3 py-1 rounded-full text-sm font-medium">
                        {getUserDisplay(message)}
                      </span>
                      <span className="bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300 px-3 py-1 rounded-full text-sm">
                        Chat: {message.chatId}
                      </span>
                      <span className="text-gray-500 dark:text-gray-400 text-sm font-mono">
                        {formatDate(message.timestamp)}
                      </span>
                      {message.replyToMessageId && (
                        <span className="bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200 px-2 py-1 rounded-full text-xs italic">
                          ↳ Reply to {message.replyToMessageId}
                        </span>
                      )}
                    </div>
                    <div className="mb-3">
                      <p className="text-gray-900 dark:text-gray-100 leading-relaxed">
                        {message.text}
                      </p>
                    </div>
                    <div className="flex justify-between items-center pt-3 border-t border-gray-100 dark:border-gray-700 text-sm">
                      <span className="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200 px-2 py-1 rounded-full text-xs capitalize">
                        {message.messageType}
                      </span>
                      <span className="text-gray-400 dark:text-gray-500 font-mono text-xs">
                        ID: {message.messageId}
                      </span>
                    </div>
                  </Card>
                ))}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
