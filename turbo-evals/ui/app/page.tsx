import Link from "next/link";
import { ArrowRight, Zap } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ComparisonTable } from "@/components/comparison-table";

export default function Home() {
  return (
    <div className="flex flex-col min-h-screen">
      <header className="border-b">
        <div className="container mx-auto px-4 lg:px-6 h-16 flex items-center max-w-7xl">
          <Link className="flex items-center justify-center" href="#">
            <Zap className="h-6 w-6 text-primary" />
            <span className="ml-2 text-xl font-bold">ModelBench</span>
          </Link>
          <nav className="ml-auto flex gap-4 sm:gap-6">
            <Link
              className="text-sm font-medium hover:underline underline-offset-4"
              href="#"
            >
              About
            </Link>
            <Link
              className="text-sm font-medium hover:underline underline-offset-4"
              href="#methodology"
            >
              Methodology
            </Link>
            <Link
              className="text-sm font-medium hover:underline underline-offset-4"
              href="#"
            >
              Contact
            </Link>
          </nav>
        </div>
      </header>
      <main className="flex-1">
        <section className="w-full py-12 md:py-24 lg:py-32 bg-muted/40">
          <div className="container mx-auto px-4 md:px-6 max-w-7xl">
            <div className="flex flex-col items-center justify-center space-y-4 text-center">
              <div className="space-y-2">
                <h1 className="text-3xl font-bold tracking-tighter sm:text-5xl xl:text-6xl/none">
                  AI Model Performance on Next.js Code
                </h1>
                <p className="max-w-[600px] text-muted-foreground md:text-xl mx-auto">
                  Comprehensive benchmarks comparing how different AI models
                  perform across 31 different evaluations when operating on
                  Next.js code.
                </p>
              </div>
              <div className="flex flex-col gap-2 min-[400px]:flex-row justify-center">
                <Button className="inline-flex h-10 items-center justify-center px-8">
                  View Benchmarks
                  <ArrowRight className="ml-2 h-4 w-4" />
                </Button>
                <Button
                  variant="outline"
                  className="inline-flex h-10 items-center justify-center px-8 bg-transparent"
                >
                  Learn More
                </Button>
              </div>
            </div>
          </div>
        </section>
        <section className="w-full py-12 md:py-24 lg:py-32">
          <div className="container mx-auto px-4 md:px-6 max-w-7xl">
            <div className="flex flex-col items-center justify-center space-y-4 text-center">
              <div className="space-y-2">
                <h2 className="text-3xl font-bold tracking-tight sm:text-4xl">
                  Model Performance Comparison
                </h2>
                <p className="max-w-[900px] text-muted-foreground md:text-xl/relaxed lg:text-base/relaxed xl:text-xl/relaxed mx-auto">
                  Compare how different AI models perform across 31 different
                  evaluations when working with Next.js code. Our benchmarks
                  measure execution time, token usage, and accuracy for specific
                  Next.js tasks.
                </p>
              </div>
            </div>
            <div className="mx-auto mt-8 w-full overflow-auto">
              <ComparisonTable />
            </div>
          </div>
        </section>
        <section
          className="w-full py-12 md:py-24 lg:py-32 bg-muted/40"
          id="methodology"
        >
          <div className="container mx-auto px-4 md:px-6 max-w-7xl">
            <div className="flex flex-col items-center justify-center space-y-4 text-center">
              <div className="space-y-2">
                <h2 className="text-3xl font-bold tracking-tight sm:text-4xl">
                  Methodology
                </h2>
                <p className="max-w-[900px] text-muted-foreground md:text-xl/relaxed lg:text-base/relaxed xl:text-xl/relaxed mx-auto">
                  Our evaluation framework provides a comprehensive and fair
                  comparison of AI models on Next.js development tasks.
                </p>
              </div>
            </div>
            <div className="mt-8 grid gap-8 md:grid-cols-2 lg:grid-cols-3">
              <Card>
                <CardContent className="p-6">
                  <h3 className="font-bold text-lg mb-2">Evaluation Tasks</h3>
                  <p className="text-sm text-muted-foreground">
                    We test 31 different Next.js tasks including routing, server
                    components, client components, API routes, data fetching,
                    state management, and more. Each task represents a
                    real-world development scenario.
                  </p>
                </CardContent>
              </Card>
              <Card>
                <CardContent className="p-6">
                  <h3 className="font-bold text-lg mb-2">
                    Performance Metrics
                  </h3>
                  <p className="text-sm text-muted-foreground">
                    We measure evaluation score (accuracy), execution duration,
                    and token usage (prompt, completion, and total). These
                    metrics provide insights into both effectiveness and
                    efficiency of each model.
                  </p>
                </CardContent>
              </Card>
              <Card>
                <CardContent className="p-6">
                  <h3 className="font-bold text-lg mb-2">Fair Comparison</h3>
                  <p className="text-sm text-muted-foreground">
                    All models receive identical prompts and are evaluated using
                    the same criteria. Results are compared against a baseline
                    to identify improvements and regressions across different
                    tasks.
                  </p>
                </CardContent>
              </Card>
              <Card>
                <CardContent className="p-6">
                  <h3 className="font-bold text-lg mb-2">
                    Evaluation Environment
                  </h3>
                  <p className="text-sm text-muted-foreground">
                    Tests are run in isolated environments to ensure
                    consistency. Each evaluation is executed with the same
                    system resources and timeout constraints to maintain
                    fairness.
                  </p>
                </CardContent>
              </Card>
              <Card>
                <CardContent className="p-6">
                  <h3 className="font-bold text-lg mb-2">Scoring System</h3>
                  <p className="text-sm text-muted-foreground">
                    Evaluation scores range from 0 to 1, where 1 indicates
                    perfect performance. Scores are calculated based on
                    successful task completion, code quality, and adherence to
                    Next.js best practices.
                  </p>
                </CardContent>
              </Card>
              <Card>
                <CardContent className="p-6">
                  <h3 className="font-bold text-lg mb-2">Continuous Updates</h3>
                  <p className="text-sm text-muted-foreground">
                    Our benchmarks are regularly updated as new model versions
                    are released. This ensures the data remains relevant and
                    reflects the current state of AI model capabilities.
                  </p>
                </CardContent>
              </Card>
            </div>
          </div>
        </section>
      </main>
      <footer className="border-t">
        <div className="container mx-auto px-4 md:px-6 max-w-7xl flex flex-col gap-2 sm:flex-row py-6">
          <p className="text-xs text-muted-foreground">
            © 2025 ModelBench. All rights reserved.
          </p>
          <nav className="sm:ml-auto flex gap-4 sm:gap-6">
            <Link
              className="text-xs hover:underline underline-offset-4"
              href="#"
            >
              Terms of Service
            </Link>
            <Link
              className="text-xs hover:underline underline-offset-4"
              href="#"
            >
              Privacy
            </Link>
          </nav>
        </div>
      </footer>
    </div>
  );
}
