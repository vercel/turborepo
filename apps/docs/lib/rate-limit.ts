import { createHash } from "node:crypto";
import { Redis } from "@upstash/redis";

type RateLimitOptions = {
  namespace: string;
  key: string;
  limit: number;
  windowSeconds: number;
};

export type RateLimitResult = {
  success: boolean;
  limit: number;
  remaining: number;
  resetAt: number;
  retryAfterSeconds: number;
};

type MemoryEntry = {
  count: number;
  resetAt: number;
};

const redis =
  process.env.UPSTASH_REDIS_REST_URL && process.env.UPSTASH_REDIS_REST_TOKEN
    ? Redis.fromEnv()
    : null;
const memoryStore = new Map<string, MemoryEntry>();

function hashKey(key: string): string {
  return createHash("sha256").update(key).digest("hex").slice(0, 32);
}

function getWindow(now: number, windowSeconds: number) {
  const windowMs = windowSeconds * 1000;
  const windowId = Math.floor(now / windowMs);

  return {
    windowId,
    resetAt: (windowId + 1) * windowMs
  };
}

function createResult(
  count: number,
  limit: number,
  resetAt: number,
  now: number
): RateLimitResult {
  return {
    success: count <= limit,
    limit,
    remaining: Math.max(0, limit - count),
    resetAt,
    retryAfterSeconds: Math.max(1, Math.ceil((resetAt - now) / 1000))
  };
}

function pruneMemoryStore(now: number): void {
  if (memoryStore.size < 10_000) {
    return;
  }

  for (const [key, entry] of memoryStore) {
    if (entry.resetAt <= now) {
      memoryStore.delete(key);
    }
  }
}

function checkMemoryRateLimit({
  namespace,
  key,
  limit,
  windowSeconds
}: RateLimitOptions): RateLimitResult {
  const now = Date.now();
  const { windowId, resetAt } = getWindow(now, windowSeconds);
  const storeKey = `${namespace}:${windowId}:${hashKey(key)}`;
  const entry = memoryStore.get(storeKey);

  pruneMemoryStore(now);

  if (!entry || entry.resetAt <= now) {
    memoryStore.set(storeKey, { count: 1, resetAt });
    return createResult(1, limit, resetAt, now);
  }

  entry.count++;
  return createResult(entry.count, limit, resetAt, now);
}

function unavailableResult(
  limit: number,
  resetAt: number,
  now: number
): RateLimitResult {
  return {
    success: false,
    limit,
    remaining: 0,
    resetAt,
    retryAfterSeconds: Math.max(1, Math.ceil((resetAt - now) / 1000))
  };
}

export async function checkRateLimit(
  options: RateLimitOptions
): Promise<RateLimitResult> {
  const now = Date.now();
  const { windowId, resetAt } = getWindow(now, options.windowSeconds);

  if (!redis) {
    if (process.env.VERCEL_ENV === "production") {
      return unavailableResult(options.limit, resetAt, now);
    }

    return checkMemoryRateLimit(options);
  }

  const redisKey = `rate-limit:${options.namespace}:${windowId}:${hashKey(
    options.key
  )}`;

  try {
    const count = await redis.incr(redisKey);

    if (count === 1) {
      await redis.expire(redisKey, options.windowSeconds * 2);
    }

    return createResult(count, options.limit, resetAt, now);
  } catch (error) {
    console.error("Rate limit check failed:", error);

    if (process.env.VERCEL_ENV === "production") {
      return unavailableResult(options.limit, resetAt, now);
    }

    return checkMemoryRateLimit(options);
  }
}
