"use client";

import { useQuery } from "convex/react";
import { api } from "../convex/_generated/api";
import { BackgroundBeams } from "./components/ui/background-beams";
import { Hero } from "./components/ui/hero";
import { StatCard } from "./components/ui/card";
import { Button } from "./components/ui/button";
import {
  MessageSquareCode,
  MessagesSquare,
  MessageSquareShare,
  DatabaseZapIcon,
} from "lucide-react";

export default function Home() {
  const messages = useQuery(api.messages.getAllMessages, { limit: 5 });
  const messageCount = messages?.length || 0;

  return (
    <div className="min-h-screen flex flex-col items-center justify-center px-4 py-20 relative">
      <BackgroundBeams />
      <main className="max-w-4xl mx-auto text-center relative z-10">
        <Hero
          title="Telegram Next.js Bot Boilerplate"
          subtitle="Monitor and view your Telegram bot messages in real-time"
        >
          {process.env.NEXT_PUBLIC_TELEGRAM_BOT_USERNAME && (
            <p className="text-lg mb-8">
              Bot username:{" "}
              <a
                href={`https://t.me/${process.env.NEXT_PUBLIC_TELEGRAM_BOT_USERNAME}`}
                target="_blank"
                rel="noopener noreferrer"
                className="text-blue-500 hover:text-blue-300 font-medium transition-colors"
              >
                t.me/{process.env.NEXT_PUBLIC_TELEGRAM_BOT_USERNAME}
              </a>
            </p>
          )}
        </Hero>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-6 mb-12">
          <StatCard
            title="Total Messages"
            value={messages === undefined ? "Loading..." : messageCount}
          />
          <StatCard
            title="Database Status"
            value={messages === undefined ? "Connecting..." : "Connected"}
          />
        </div>

        <div className="flex flex-wrap gap-4 justify-center">
          <Button href="/messages" variant="secondary">
            <MessageSquareCode className="w-4 h-4 mr-2" />
            View Messages
          </Button>
          <Button href="/threads" variant="secondary">
            <MessagesSquare className="w-4 h-4 mr-2" />
            Browse Threads
          </Button>
          <Button href="/send" variant="secondary">
            <MessageSquareShare className="w-4 h-4 mr-2" />
            Send Message
          </Button>
          <Button href="/convex-web-console-directions" variant="secondary">
            <DatabaseZapIcon className="w-4 h-4 mr-2" />
            Convex Console
          </Button>
        </div>
      </main>
    </div>
  );
}
