"use client";

import { Card } from "../components/ui/card";
import { Button } from "../components/ui/button";
import { Hero } from "../components/ui/hero";
import {
  Accordion,
  AccordionItem,
  AccordionTrigger,
  AccordionContent,
} from "../components/ui/accordion";
import { BackgroundGradient } from "../components/ui/background-gradient";
import { Timeline } from "../components/ui/timeline";
import { AuroraBackground } from "../components/ui/aurora-background";
import { cn } from "../lib/utils";
import {
  Database,
  ExternalLink,
  Info,
  ArrowRight,
  Terminal,
  Key,
  Eye,
  EyeOff,
  Copy,
  Check,
} from "lucide-react";
import { useState, useEffect } from "react";

export default function ConvexWebConsoleDirections() {
  const [showAdminKey, setShowAdminKey] = useState(false);
  const [copied, setCopied] = useState(false);
  const [showAurora, setShowAurora] = useState(true);
  const [scrollY, setScrollY] = useState(0);
  const dashboardPort = process.env.NEXT_PUBLIC_CONVEX_DASHBOARD_PORT || "6791";
  const convexPort = process.env.NEXT_PUBLIC_CONVEX_PORT || "3210";
  const dashboardUrl = `http://localhost:${dashboardPort}`;
  const deploymentUrl = `http://localhost:${convexPort}`;

  useEffect(() => {
    const handleScroll = () => {
      const currentScrollY = window.scrollY;
      setScrollY(currentScrollY);

      // Gradually fade aurora as user scrolls, completely hide after 120vh
      const heroHeight =
        typeof window !== "undefined" ? window.innerHeight : 800;
      setShowAurora(currentScrollY < heroHeight * 1.2);
    };

    if (typeof window !== "undefined") {
      window.addEventListener("scroll", handleScroll, { passive: true });
      return () => window.removeEventListener("scroll", handleScroll);
    }
  }, []);

  const timelineData = [
    {
      title: "Step 1",
      content: (
        <Card className="bg-gray-900/90 border-gray-700/50">
          <div className="text-left">
            <div className="flex items-center mb-4">
              <Terminal className="w-6 h-6 text-curious-blue-500 mr-2" />
              <h3 className="text-xl font-semibold text-gray-900 dark:text-white">
                Get Admin Key
              </h3>
            </div>
            <p className="text-gray-600 dark:text-gray-300 mb-4">
              First, navigate to your project root directory (where you cloned
              the repo) and generate an admin key (when your docker convex app
              is running):
            </p>
            <div className="space-y-3 mb-4">
              <div className="bg-gray-100 dark:bg-gray-800 rounded-lg p-4">
                <div className="flex items-center justify-between">
                  <code className="text-sm text-gray-800 dark:text-gray-200 flex-1">
                    cd /path/to/go-convex-telegram-turborepo
                  </code>
                </div>
              </div>
              <div className="bg-gray-100 dark:bg-gray-800 rounded-lg p-4">
                <div className="flex items-center justify-between">
                  <code className="text-sm text-gray-800 dark:text-gray-200 flex-1">
                    pnpm run get-admin-key
                  </code>
                  <button
                    onClick={() => {
                      navigator.clipboard.writeText("npm run get-admin-key");
                      setCopied(true);
                      setTimeout(() => setCopied(false), 2000);
                    }}
                    className="ml-2 p-1 text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 transition-colors"
                    title="Copy command"
                  >
                    {copied ? (
                      <Check className="w-4 h-4 text-green-500" />
                    ) : (
                      <Copy className="w-4 h-4" />
                    )}
                  </button>
                </div>
              </div>
            </div>
            <p className="text-sm text-gray-500 dark:text-gray-400">
              This will generate a unique admin key that you&apos;ll need to
              access the dashboard.
            </p>
          </div>
        </Card>
      ),
    },
    {
      title: "Step 2",
      content: (
        <div className="text-left w-full">
          <Card className="mb-2 bg-gray-900/90 border-gray-700/50">
            <div className="flex items-center justify-center -py-4 rounded-3xl">
              <h3 className="text-xl font-semibold text-white">
                Login to Dashboard
              </h3>
            </div>
          </Card>

          <BackgroundGradient className="rounded-[22px] p-6 bg-white dark:bg-zinc-900">
            {/* Convex Logo Card */}
            <Card className="mb-2 bg-gray-900/90 border-gray-700/50">
              <div className="flex items-center justify-center -py-4">
                <img
                  src="https://docs.convex.dev/img/convex-dark.svg"
                  alt="Convex Logo"
                  className="h-8"
                />
              </div>
            </Card>

            {/* Deployment URL Card */}
            <Card className="mb-4">
              <div className="text-left">
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Deployment URL
                </label>
                <div className="bg-gray-50 dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2">
                  <code className="text-sm text-gray-800 dark:text-gray-200">
                    {deploymentUrl}
                  </code>
                </div>
              </div>
            </Card>

            {/* Admin Key Card */}
            <Card className="mb-4">
              <div className="text-left">
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Admin Key
                </label>
                <div className="relative">
                  <div className="bg-gray-50 dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-lg px-3 py-2 pr-10">
                    <code className="text-sm text-gray-800 dark:text-gray-200">
                      {showAdminKey
                        ? "your-generated-admin-key-here"
                        : "••••••••••••••••••••••••••••"}
                    </code>
                  </div>
                  <button
                    onClick={() => setShowAdminKey(!showAdminKey)}
                    className="absolute right-2 top-1/2 transform -translate-y-1/2 text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
                  >
                    {showAdminKey ? (
                      <EyeOff className="w-4 h-4" />
                    ) : (
                      <Eye className="w-4 h-4" />
                    )}
                  </button>
                </div>
                <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                  The admin key is required every time you open the dashboard.
                </p>
              </div>
            </Card>

            {/* Login Button Card */}
            <Card className="">
              <Button
                href={dashboardUrl}
                variant="secondary"
                className="w-full bg-curious-blue-600 hover:bg-curious-blue-700 text-white border-curious-blue-600"
              >
                <Key className="w-4 h-4 mr-2" />
                Log In
              </Button>
            </Card>
          </BackgroundGradient>
        </div>
      ),
    },
    {
      title: "Step 3",
      content: (
        <div className="text-left w-full">
          <Card className="mb-2 bg-gray-900/90 border-gray-700/50">
            <div className="flex items-center justify-center -py-4 rounded-2xl">
              <h3 className="text-xl font-semibold text-white">
                What You&apos;ll See After Login
              </h3>
            </div>
          </Card>
          <div className="max-w-2xl mx-auto">
            <BackgroundGradient className="rounded-[22px] p-6 bg-white dark:bg-zinc-900">
              {/* Dashboard Header */}
              <Card className="mb-4 bg-curious-blue-950 border-curious-blue-800">
                <div className="flex items-center justify-between">
                  <div className="flex items-center">
                    <img
                      src="https://docs.convex.dev/img/convex-dark.svg"
                      alt="Convex Logo"
                      className="h-6 mr-3"
                    />
                    <span className="text-white font-semibold">
                      Convex Dashboard
                    </span>
                  </div>
                  <div className="text-curious-blue-300 text-sm">Health: ✓</div>
                </div>
              </Card>

              {/* Navigation Tabs */}
              <Card className="mb-4">
                <div className="flex flex-wrap gap-1 bg-gray-100 dark:bg-gray-800 rounded-lg p-1">
                  <div className="bg-curious-blue-600 text-white px-3 py-1 rounded text-sm font-medium">
                    Tables
                  </div>
                  <div className="text-gray-600 dark:text-gray-400 px-3 py-1 rounded text-sm">
                    Data
                  </div>
                  <div className="text-gray-600 dark:text-gray-400 px-3 py-1 rounded text-sm">
                    Functions
                  </div>
                  <div className="text-gray-600 dark:text-gray-400 px-3 py-1 rounded text-sm">
                    Files
                  </div>
                  <div className="text-gray-600 dark:text-gray-400 px-3 py-1 rounded text-sm">
                    Schedules
                  </div>
                  <div className="text-gray-600 dark:text-gray-400 px-3 py-1 rounded text-sm">
                    Logs
                  </div>
                </div>
              </Card>

              {/* Tables List */}
              <Card className="mb-4">
                <div className="space-y-2">
                  <div className="flex items-center justify-between py-2 border-b border-gray-200 dark:border-gray-700">
                    <div className="flex items-center min-w-0 flex-1">
                      <Database className="w-4 h-4 text-curious-blue-500 mr-2 flex-shrink-0" />
                      <span className="font-medium text-gray-900 dark:text-white truncate">
                        telegram_messages
                      </span>
                    </div>
                    <span className="text-sm text-gray-500 dark:text-gray-400 ml-2 flex-shrink-0">
                      5 documents
                    </span>
                  </div>
                  <div className="flex items-center justify-between py-2 border-b border-gray-200 dark:border-gray-700">
                    <div className="flex items-center min-w-0 flex-1">
                      <Database className="w-4 h-4 text-curious-blue-500 mr-2 flex-shrink-0" />
                      <span className="font-medium text-gray-900 dark:text-white truncate">
                        telegram_threads
                      </span>
                    </div>
                    <span className="text-sm text-gray-500 dark:text-gray-400 ml-2 flex-shrink-0">
                      1 document
                    </span>
                  </div>
                  <div className="flex items-center justify-between py-2">
                    <div className="flex items-center min-w-0 flex-1">
                      <Database className="w-4 h-4 text-curious-blue-500 mr-2 flex-shrink-0" />
                      <span className="font-medium text-gray-900 dark:text-white truncate">
                        + Create Table
                      </span>
                    </div>
                    <ArrowRight className="w-4 h-4 text-gray-400 ml-2 flex-shrink-0" />
                  </div>
                </div>
              </Card>

              {/* Action Buttons */}
              <div className="grid grid-cols-2 gap-3">
                <Card className="bg-curious-blue-50 dark:bg-curious-blue-950 border-curious-blue-200 dark:border-curious-blue-800">
                  <div className="text-center">
                    <Database className="w-6 h-6 text-curious-blue-600 dark:text-curious-blue-400 mx-auto mb-2" />
                    <p className="text-sm font-medium text-curious-blue-900 dark:text-curious-blue-100">
                      Browse Data
                    </p>
                    <p className="text-xs text-curious-blue-700 dark:text-curious-blue-300">
                      View & edit records
                    </p>
                  </div>
                </Card>
                <Card className="bg-curious-blue-50 dark:bg-curious-blue-950 border-curious-blue-200 dark:border-curious-blue-800">
                  <div className="text-center">
                    <Terminal className="w-6 h-6 text-curious-blue-600 dark:text-curious-blue-400 mx-auto mb-2" />
                    <p className="text-sm font-medium text-curious-blue-900 dark:text-curious-blue-100">
                      Function Logs
                    </p>
                    <p className="text-xs text-curious-blue-700 dark:text-curious-blue-300">
                      Monitor activity
                    </p>
                  </div>
                </Card>
              </div>
            </BackgroundGradient>
          </div>
        </div>
      ),
    },
  ];

  return (
    <div className="relative min-h-screen">
      {/* Aurora Background - Full Screen Behind Everything with Parallax */}
      {showAurora && (
        <AuroraBackground
          showRadialGradient={true}
          className="fixed inset-0 z-0"
          style={{
            transform: `translateY(${-scrollY * 0.3}px)`,
            opacity:
              typeof window !== "undefined"
                ? Math.max(0, 1 - scrollY / (window.innerHeight * 1.0))
                : 1,
            transition: "opacity 0.3s ease-out",
          }}
        >
          <div></div>
        </AuroraBackground>
      )}

      {/* Grid Background with Gradient Transition */}
      <div
        className={cn(
          "absolute inset-0 z-10 transition-all duration-700 ease-out",
          "[background-size:40px_40px]"
        )}
        style={{
          backgroundImage: (() => {
            const scrollProgress =
              scrollY /
              (typeof window !== "undefined" ? window.innerHeight : 800);
            const opacity = 0.2 + scrollProgress * 0.4;

            if (showAurora) {
              return `linear-gradient(to right, rgba(228,228,231,${
                opacity * 0.5
              }) 1px, transparent 1px), linear-gradient(to bottom, rgba(228,228,231,${
                opacity * 0.5
              }) 1px, transparent 1px)`;
            }
            return "linear-gradient(to right, rgba(228,228,231,0.4) 1px, transparent 1px), linear-gradient(to bottom, rgba(228,228,231,0.4) 1px, transparent 1px)";
          })(),
          maskImage: showAurora
            ? `linear-gradient(to bottom, transparent 0%, rgba(0,0,0,${
                0.1 +
                (scrollY /
                  (typeof window !== "undefined" ? window.innerHeight : 800)) *
                  0.3
              }) 20%, rgba(0,0,0,${
                0.6 +
                (scrollY /
                  (typeof window !== "undefined" ? window.innerHeight : 800)) *
                  0.4
              }) 60%, black 100%)`
            : "none",
          backgroundColor: showAurora
            ? "transparent"
            : "rgba(255,255,255,0.05)",
        }}
      />

      {/* Content Container */}
      <div className="relative z-20 min-h-screen flex flex-col items-center justify-center px-4 pt-24 pb-20">
        {/* Hero Section */}
        <div className="max-w-4xl mx-auto text-center mb-8">
          <Hero
            title="Convex Web Console"
            subtitle="Access your Convex database dashboard and manage your data"
            className="mb-8"
            whiteText
          />
        </div>

        <main className="max-w-4xl mx-auto text-center">
          {/* Prerequisites Accordion */}
          <div className="mb-8">
            <Accordion
              type="single"
              collapsible
              className="w-full pl-8 pr-8 hover:border-white hover:border-2 border-2 border-white/10 rounded-xl"
            >
              <AccordionItem value="prerequisites">
                <AccordionTrigger className="text-left hover:no-underline group">
                  <div className="flex items-center justify-between w-full">
                    <div className="flex items-center">
                      <Info className="w-5 h-5 text-curious-blue-600 dark:text-curious-blue-400 mr-3" />
                      <span className="text-lg font-semibold text-gray-900 dark:text-white">
                        Prerequisites
                      </span>
                      <div className="ml-3 h-px bg-gray-300 dark:bg-gray-600 flex-1 max-w-[100px] group-hover:bg-curious-blue-400 transition-colors"></div>
                    </div>
                  </div>
                </AccordionTrigger>
                <AccordionContent>
                  <div className="text-left space-y-3 pt-4">
                    <p className="text-gray-900 dark:text-white">
                      <strong>Important:</strong> These directions only work if
                      you have the complete Docker setup running. The Convex
                      dashboard requires both Convex containers to be active:
                    </p>
                    <ul className="list-disc list-inside space-y-2 text-gray-700 dark:text-gray-300 ml-4">
                      <li>
                        <strong>Convex Backend Container</strong> - The
                        self-hosted Convex database (port {convexPort})
                      </li>
                      <li>
                        <strong>Convex Dashboard Container</strong> - The
                        web-based management interface (port {dashboardPort})
                      </li>
                    </ul>
                    <div className="rounded-lg p-4 mt-4">
                      <p className="text-sm text-gray-900 dark:text-white mb-2">
                        <strong>To start the required services:</strong>
                      </p>
                      <div className="flex items-center justify-between">
                        <code className="text-sm text-gray-800 dark:text-gray-200 flex-1 bg-gray-100 dark:bg-gray-800">
                          pnpm setup-init
                        </code>
                        <button
                          onClick={() => {
                            navigator.clipboard.writeText(
                              "npm run get-admin-key"
                            );
                            setCopied(true);
                            setTimeout(() => setCopied(false), 2000);
                          }}
                          className="ml-2 p-1 text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 transition-colors"
                          title="Copy command"
                        >
                          {copied ? (
                            <Check className="w-4 h-4 text-green-500" />
                          ) : (
                            <Copy className="w-4 h-4" />
                          )}
                        </button>
                      </div>
                      <p className="text-xs text-gray-600 dark:text-gray-400 mt-2">
                        Run this command from the project root directory to
                        start all Docker containers.
                      </p>
                    </div>
                  </div>
                </AccordionContent>
              </AccordionItem>
            </Accordion>
          </div>

          {/* Dashboard Access Cards */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6 mb-8">
            <Card className="text-left">
              <div className="flex items-center mb-4">
                <Info className="w-6 h-6 text-curious-blue-500 mr-2" />
                <h3 className="text-xl font-semibold text-gray-900 dark:text-white">
                  Dashboard Access
                </h3>
              </div>
              <p className="text-gray-600 dark:text-gray-300 mb-4">
                The Convex Dashboard is available on port{" "}
                <span className="font-mono bg-curious-blue-100 dark:bg-curious-blue-900 px-2 py-1 rounded text-curious-blue-800 dark:text-curious-blue-200">
                  {dashboardPort}
                </span>
              </p>
              <Button
                href={dashboardUrl}
                variant="secondary"
                className="w-full"
              >
                <ExternalLink className="w-4 h-4 mr-2" />
                Open Dashboard
              </Button>
            </Card>

            <Card className="text-left">
              <div className="flex items-center mb-4">
                <Database className="w-6 h-6 text-curious-blue-500 mr-2" />
                <h3 className="text-xl font-semibold text-gray-900 dark:text-white">
                  Deployment URL
                </h3>
              </div>
              <p className="text-gray-600 dark:text-gray-300 mb-4">
                Your Convex deployment is running on port{" "}
                <span className="font-mono bg-curious-blue-100 dark:bg-curious-blue-900 px-2 py-1 rounded text-curious-blue-800 dark:text-curious-blue-200">
                  {convexPort}
                </span>
              </p>
              <div className="text-sm text-gray-500 dark:text-gray-400">
                <code className="bg-gray-100 dark:bg-gray-800 px-2 py-1 rounded">
                  {deploymentUrl}
                </code>
              </div>
            </Card>
          </div>

          {/* Timeline Steps */}
          <Timeline data={timelineData} />

          <Card className="text-left mt-6 max-w-2xl mx-auto mb-12">
            <h4 className="text-lg font-semibold text-gray-900 dark:text-white mb-3">
              Why Convex is Powerful
            </h4>
            <div className="space-y-3">
              <div className="flex items-start">
                <ArrowRight className="w-5 h-5 text-curious-blue-500 mr-3 mt-0.5 flex-shrink-0" />
                <div className="min-w-0">
                  <p className="font-medium text-gray-900 dark:text-white">
                    Data Management
                  </p>
                  <p className="text-gray-600 dark:text-gray-300 text-sm">
                    Convex comes with a built in dashboard to manage your data
                    without an external daatabase viewer.
                  </p>
                </div>
              </div>
              <div className="flex items-start">
                <ArrowRight className="w-5 h-5 text-curious-blue-500 mr-3 mt-0.5 flex-shrink-0" />
                <div className="min-w-0">
                  <p className="font-medium text-gray-900 dark:text-white">
                    Real Time Data Loading
                  </p>
                  <p className="text-gray-600 dark:text-gray-300 text-sm">
                    Convex comes with a built in web socket for react apps so
                    you do not have to build one to enable real time stateful
                    data loading.
                  </p>
                </div>
              </div>
              <div className="flex items-start">
                <ArrowRight className="w-5 h-5 text-curious-blue-500 mr-3 mt-0.5 flex-shrink-0" />
                <div className="min-w-0">
                  <p className="font-medium text-gray-900 dark:text-white">
                    Schema Visualization
                  </p>
                  <p className="text-gray-600 dark:text-gray-300 text-sm">
                    Understand your data structure and relationships at a glance
                  </p>
                </div>
              </div>
            </div>
          </Card>

          <div className="bg-curious-blue-50 dark:bg-curious-blue-950 border border-curious-blue-200 dark:border-curious-blue-800 rounded-xl p-6">
            <div className="flex items-center justify-center mb-2">
              <Info className="w-5 h-5 text-curious-blue-600 dark:text-curious-blue-400 mr-2" />
              <h4 className="font-semibold text-curious-blue-900 dark:text-curious-blue-100">
                Need Help?
              </h4>
            </div>
            <p className="text-curious-blue-700 dark:text-curious-blue-300 text-sm">
              Visit the{" "}
              <a
                href="https://docs.convex.dev"
                target="_blank"
                rel="noopener noreferrer"
                className="underline hover:text-curious-blue-600 dark:hover:text-curious-blue-200"
              >
                Convex Documentation
              </a>{" "}
              for detailed guides and tutorials.
            </p>
          </div>
        </main>
      </div>
    </div>
  );
}
