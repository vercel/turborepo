"use client";

import { usePathname, useRouter } from "next/navigation";
import { Tabs } from "./ui/tabs";
import {
  Bot,
  HouseWifi,
  MessageSquareCode,
  MessagesSquare,
  MessageSquareShare,
  DatabaseZapIcon,
} from "lucide-react";
import { useEffect, useState } from "react";

export default function Navigation() {
  const pathname = usePathname();
  const router = useRouter();
  const [isScrolled, setIsScrolled] = useState(false);

  useEffect(() => {
    const handleScroll = () => {
      const scrollPosition = window.scrollY;
      setIsScrolled(scrollPosition > 10);
    };

    window.addEventListener("scroll", handleScroll);
    return () => window.removeEventListener("scroll", handleScroll);
  }, []);

  const navItems = [
    { href: "/", label: "Home", icon: HouseWifi },
    { href: "/messages", label: "Messages", icon: MessageSquareCode },
    { href: "/threads", label: "Threads", icon: MessagesSquare },
    { href: "/send", label: "Send Message", icon: MessageSquareShare },
    {
      href: "/convex-web-console-directions",
      label: "Console",
      icon: DatabaseZapIcon,
    },
  ];

  // Convert nav items to tabs format
  const tabs = navItems.map((item) => {
    const IconComponent = item.icon;
    return {
      title: (
        <div className="flex items-center gap-2">
          <IconComponent className="w-4 h-4" />
          <span
            className={`hidden md:inline ${
              isScrolled && item.href !== pathname ? "lg:hidden" : ""
            }`}
          >
            {item.label}
          </span>
        </div>
      ),
      value: item.href,
      content: (
        <div className="flex items-center gap-2">
          <IconComponent className="w-4 h-4" />
          <span className="hidden md:inline">{item.label}</span>
        </div>
      ),
    };
  });

  // Find current active tab based on pathname
  const activeTabIndex = navItems.findIndex((item) => item.href === pathname);

  return (
    <nav className="fixed top-0 left-0 right-0 z-50 ">
      <div className="max-w-6xl mx-auto px-4 flex items-center justify-between h-16">
        <div className="flex items-center gap-2 font-semibold text-blue-500">
          <Bot className="w-6 h-6" />
          <span
            className={`text-lg font-bold bg-gradient-to-r from-blue-500 to-blue-100 bg-clip-text text-transparent hidden ${
              isScrolled ? "sm:hidden" : "sm:inline"
            }`}
          >
            Bot Manager
          </span>
        </div>

        <div className="flex items-center">
          <Tabs
            tabs={tabs}
            activeTabIndex={activeTabIndex}
            containerClassName="flex-1 justify-center"
            activeTabClassName="bg-blue-500 text-white rounded-lg"
            tabClassName="text-white/80 hover:text-blue-300 transition-colors duration-200 font-medium px-3 py-2 rounded-lg"
            contentClassName="hidden"
            onTabChange={(tab) => {
              router.push(tab.value);
            }}
          />
        </div>

        <div className="flex items-center">
          {/* Database icon moved to nav items */}
        </div>
      </div>
    </nav>
  );
}
