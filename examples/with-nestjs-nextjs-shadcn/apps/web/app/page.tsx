import { Button } from "@repo/ui/components/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@repo/ui/components/card";
import { Badge } from "@repo/ui/components/badge";
import { Input } from "@repo/ui/components/input";
import { Github, Zap, Database, Globe } from "lucide-react";

export default function Page() {
  return (
    <div className="min-h-screen bg-white">
      {/* Simple Hero */}
      <div className="container mx-auto px-6 py-24">
        <div className="text-center max-w-3xl mx-auto">
          <Badge variant="outline" className="mb-6">
            <Zap className="w-3 h-3 mr-1" />
            Turborepo Template
          </Badge>

          <h1 className="text-5xl font-bold text-gray-900 mb-6">Next.js + NestJS + ShadCN</h1>

          <p className="text-lg text-gray-600 mb-8 leading-relaxed">
            A modern monorepo template with everything you need to build full-stack applications.
          </p>

          <div className="flex gap-3 justify-center">
            <Button size="lg">Get Started</Button>
            <Button variant="outline" size="lg">
              <Github className="w-4 h-4 mr-2" />
              GitHub
            </Button>
          </div>
        </div>
      </div>

      {/* Simple Features */}
      <div className="bg-gray-50 py-20">
        <div className="container mx-auto px-6">
          <div className="grid md:grid-cols-3 gap-8">
            <Card className="border-0 shadow-sm">
              <CardHeader>
                <div className="w-10 h-10 bg-blue-100 rounded-lg flex items-center justify-center mb-4">
                  <Globe className="w-5 h-5 text-blue-600" />
                </div>
                <CardTitle>Next.js Frontend</CardTitle>
                <CardDescription>Modern React with App Router and TypeScript</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex gap-2 flex-wrap">
                  <Badge variant="secondary">App Router</Badge>
                  <Badge variant="secondary">TypeScript</Badge>
                  <Badge variant="secondary">Tailwind</Badge>
                </div>
              </CardContent>
            </Card>

            <Card className="border-0 shadow-sm">
              <CardHeader>
                <div className="w-10 h-10 bg-red-100 rounded-lg flex items-center justify-center mb-4">
                  <Database className="w-5 h-5 text-red-600" />
                </div>
                <CardTitle>NestJS Backend</CardTitle>
                <CardDescription>Scalable Node.js framework with TypeScript</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex gap-2 flex-wrap">
                  <Badge variant="secondary">TypeScript</Badge>
                  <Badge variant="secondary">Decorators</Badge>
                  <Badge variant="secondary">OpenAPI</Badge>
                </div>
              </CardContent>
            </Card>

            <Card className="border-0 shadow-sm">
              <CardHeader>
                <div className="w-10 h-10 bg-purple-100 rounded-lg flex items-center justify-center mb-4">
                  <Zap className="w-5 h-5 text-purple-600" />
                </div>
                <CardTitle>Turborepo</CardTitle>
                <CardDescription>High-performance build system for monorepos</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex gap-2 flex-wrap">
                  <Badge variant="secondary">Caching</Badge>
                  <Badge variant="secondary">Parallel</Badge>
                  <Badge variant="secondary">Incremental</Badge>
                </div>
              </CardContent>
            </Card>
          </div>
        </div>
      </div>

      {/* Simple Demo */}
      <div className="py-20">
        <div className="container mx-auto px-6">
          <div className="max-w-2xl mx-auto">
            <div className="text-center mb-12">
              <h2 className="text-3xl font-bold text-gray-900 mb-4">UI Components</h2>
              <p className="text-gray-600">Built with ShadCN UI and Tailwind CSS</p>
            </div>

            <Card className="border shadow-sm">
              <CardHeader>
                <CardTitle>Component Examples</CardTitle>
                <CardDescription>Try the available components</CardDescription>
              </CardHeader>
              <CardContent className="space-y-6">
                <div>
                  <label className="text-sm font-medium mb-2 block">Input</label>
                  <Input placeholder="Type something..." />
                </div>

                <div>
                  <label className="text-sm font-medium mb-2 block">Buttons</label>
                  <div className="flex gap-2 flex-wrap">
                    <Button>Primary</Button>
                    <Button variant="secondary">Secondary</Button>
                    <Button variant="outline">Outline</Button>
                  </div>
                </div>

                <div>
                  <label className="text-sm font-medium mb-2 block">Badges</label>
                  <div className="flex gap-2 flex-wrap">
                    <Badge>Default</Badge>
                    <Badge variant="secondary">Secondary</Badge>
                    <Badge variant="outline">Outline</Badge>
                  </div>
                </div>
              </CardContent>
            </Card>
          </div>
        </div>
      </div>

      {/* Simple Footer */}
      <div className="border-t py-12">
        <div className="container mx-auto px-6 text-center">
          <p className="text-gray-600 mb-4">Ready to build something amazing?</p>
          <Button>Start Building</Button>
        </div>
      </div>
    </div>
  );
}
