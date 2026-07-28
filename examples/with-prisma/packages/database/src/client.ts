import { PrismaPg } from '@prisma/adapter-pg';
import { Pool } from 'pg'; // Import the Pool
import { PrismaClient } from "../generated/client";

const globalForPrisma = global as unknown as { prisma: PrismaClient };

// 1. Create the Pool specifically for the adapter
const connectionString = process.env.DATABASE_URL;

const pool = new Pool({
  connectionString
});

// 2. Pass the pool to the adapter
const adapter = new PrismaPg(pool);

export const prisma =
  globalForPrisma.prisma ||
  new PrismaClient({
    adapter,
    // Optional: Log queries to see if connection works
    // log: ['query', 'info', 'warn', 'error'],
  });

if (process.env.NODE_ENV !== "production") globalForPrisma.prisma = prisma;

export * from "../generated/client";
