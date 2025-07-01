"use client";

import { useQuery } from "convex/react";
import { api } from "../../convex/_generated/api";
import { TelegramThread } from "../models/telegram";
import { Hero } from "../components/ui/hero";
import { Card } from "../components/ui/card";
import ThreadModal from "../components/ThreadModal";
import { useState } from "react";

export default function ThreadsPage() {
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(null);
  const threads = useQuery(api.threads.getAllActiveThreads, { limit: 50 });

  const handleThreadClick = (threadId: string) => {
    setSelectedThreadId(threadId);
  };

  const handleCloseModal = () => {
    setSelectedThreadId(null);
  };

  if (threads === undefined) {
    return (
      <div className="max-w-6xl mx-auto p-6">
        <div className="text-center py-16">
          <div className="text-xl text-gray-600 dark:text-gray-300">
            Loading threads...
          </div>
        </div>
      </div>
    );
  }

  if (threads.length === 0) {
    return (
      <div className="max-w-6xl mx-auto p-6">
        <Hero
          title="Telegram Threads"
          subtitle="Manage your group conversations"
        />
        <Card className="text-center py-12">
          <p className="text-gray-600 dark:text-gray-400 mb-2">
            No threads found.
          </p>
          <p className="text-gray-500 dark:text-gray-500">
            Threads will appear here when messages are sent in Telegram group
            threads.
          </p>
        </Card>
      </div>
    );
  }

  return (
    <div className="max-w-6xl mx-auto p-6">
      <Hero
        title="Telegram Threads"
        subtitle={`${threads.length} active threads`}
      />
      <div className="flex flex-col gap-4">
        {threads.map((thread: TelegramThread) => (
          <div
            key={thread._id}
            className="cursor-pointer transition-all duration-200 hover:shadow-lg hover:-translate-y-1"
            onClick={() => handleThreadClick(thread._id)}
          >
            <Card>
              <div className="flex justify-between items-start mb-4">
                <h3 className="text-xl font-semibold text-gray-900 dark:text-white">
                  {thread.title || `Thread ${thread.threadId}`}
                </h3>
                <span className="bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300 px-3 py-1 rounded-full text-sm">
                  Chat: {thread.chatId}
                </span>
              </div>

              <div className="space-y-3 mb-4">
                <div className="flex flex-wrap gap-2">
                  {thread.creatorFirstName && (
                    <span className="text-gray-600 dark:text-gray-400 text-sm">
                      Created by: {thread.creatorFirstName}{" "}
                      {thread.creatorLastName}
                    </span>
                  )}
                  {thread.creatorUsername && (
                    <span className="bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200 px-2 py-1 rounded-full text-xs">
                      @{thread.creatorUsername}
                    </span>
                  )}
                </div>
                <div className="flex flex-wrap gap-4 text-sm text-gray-600 dark:text-gray-400">
                  <span className="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200 px-2 py-1 rounded-full text-xs">
                    {thread.messageCount} messages
                  </span>
                  <span>
                    Last activity:{" "}
                    {thread.lastMessageTimestamp
                      ? new Date(thread.lastMessageTimestamp).toLocaleString()
                      : "Unknown"}
                  </span>
                </div>
              </div>

              {thread.lastMessageText && (
                <div className="mb-4 p-3 bg-gray-50 dark:bg-gray-800 rounded-lg">
                  <p className="text-gray-700 dark:text-gray-300 text-sm italic">
                    {thread.lastMessageText.length > 100
                      ? `${thread.lastMessageText.substring(0, 100)}...`
                      : thread.lastMessageText}
                  </p>
                </div>
              )}

              <div className="flex justify-between items-center pt-3 border-t border-gray-100 dark:border-gray-700">
                <span className="text-gray-400 dark:text-gray-500 font-mono text-xs">
                  Thread ID: {thread.threadId}
                </span>
                <span
                  className={`px-2 py-1 rounded-full text-xs font-medium ${
                    thread.isActive
                      ? "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200"
                      : "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200"
                  }`}
                >
                  {thread.isActive ? "Active" : "Inactive"}
                </span>
              </div>
            </Card>
          </div>
        ))}
      </div>

      <ThreadModal
        threadId={selectedThreadId || ""}
        isOpen={selectedThreadId !== null}
        onClose={handleCloseModal}
      />
    </div>
  );
}
