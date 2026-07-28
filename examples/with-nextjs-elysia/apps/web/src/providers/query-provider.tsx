"use client";

import {
  MutationCache, // 🚨 新增：处理 Post/Put/Delete 错误
  QueryCache,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import React from "react";
import { toast } from "sonner";

/**
 * 定义 RFC 9457 错误结构
 */
interface ProblemDetails {
  title: string;
  status: number;
  detail?: string;
  instance?: string;
  [key: string]: any; // 支持 x-pg-code 等扩展字段
}

export default function QueryProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  // 统一错误处理函数
  const handleGlobalError = async (error: any) => {
    let errorMessage = "发生未知错误";
    let errorTitle = "请求失败";

    // 1. 尝试解析符合 RFC 标准的错误响应
    // Eden Treaty 错误结构: { error: { status, title, detail, ... } }
    const edenError = error?.error;
    // 其他可能的错误结构
    const problem =
      edenError || error?.response?.data || error?.response || error;

    if (problem && typeof problem === "object" && "title" in problem) {
      const p = problem as ProblemDetails;
      errorTitle = p.title;
      // 优先显示具体细节 detail，没有则显示 title
      errorMessage = p.detail || p.title;

      // 可以在这里针对特定的业务错误码做特殊处理
      if (p["x-pg-code"] === "23503") {
        console.warn("数据库外键约束冲突:", p["x-constraint"]);
      }
    } else {
      // 兜底逻辑
      errorMessage = error instanceof Error ? error.message : String(error);
    }

    // 2. 弹出 UI 提示
    toast.error(errorTitle, {
      description: errorMessage,
      duration: 4000,
    });

    console.error(`[Global API Error] ${errorTitle}:`, problem);
  };

  const [queryClient] = React.useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            refetchOnWindowFocus: false,
            retry: 1, // 失败重试次数
          },
        },
        // 处理 GET 请求错误
        queryCache: new QueryCache({
          onError: (error) => handleGlobalError(error),
        }),
        // 处理 POST/PUT/DELETE 请求错误
        mutationCache: new MutationCache({
          onError: (error) => handleGlobalError(error),
        }),
      })
  );

  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}
