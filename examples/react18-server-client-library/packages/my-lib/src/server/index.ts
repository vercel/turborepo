/**
 * need to export server components and client components from separate files as
 * directive on top of the file from which component is imported takes effect.
 * i.e., server component re-exported from file with "use client" will behave as client component
 * */

// server component exports
export * from "./card";
