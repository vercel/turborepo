/**
 * Turbo Mode System - Main Entry Point
 * 
 * This module provides a comprehensive turbo mode system that can speed up
 * application operations by a configurable multiplier (default 2x).
 * 
 * Features:
 * - Toggle turbo mode on/off
 * - Configurable speed multipliers
 * - Performance monitoring and metrics
 * - Event-driven architecture
 * - Utility functions for common optimizations
 * - TypeScript support with full type safety
 */

// Core exports
export { TurboManager } from './core/turbo-manager';

// Configuration exports
export { TurboConfigManager, DEFAULT_TURBO_CONFIG } from './config/turbo-config';

// Utility exports
export { PerformanceUtils } from './utils/performance-utils';

// Type exports
export type {
  TurboConfig,
  TurboEvents,
  PerformanceMetrics,
  TurboStateCallback,
  OptimizationOptions,
} from './types/turbo-types';

// Import for internal use
import { TurboManager } from './core/turbo-manager';
import type { TurboConfig } from './types/turbo-types';

// Convenience function to get a configured turbo manager instance
export function createTurboManager(config?: Partial<TurboConfig>) {
  return TurboManager.getInstance(config);
}

// Quick access to common turbo operations
export const turbo = {
  /**
   * Get the default turbo manager instance
   */
  getInstance: () => TurboManager.getInstance(),
  
  /**
   * Enable turbo mode
   */
  enable: () => TurboManager.getInstance().enable(),
  
  /**
   * Disable turbo mode
   */
  disable: () => TurboManager.getInstance().disable(),
  
  /**
   * Toggle turbo mode
   */
  toggle: () => TurboManager.getInstance().toggle(),
  
  /**
   * Check if turbo mode is enabled
   */
  isEnabled: () => TurboManager.getInstance().isEnabledState(),
  
  /**
   * Get current speed multiplier
   */
  getSpeedMultiplier: () => TurboManager.getInstance().getSpeedMultiplier(),
  
  /**
   * Get performance metrics
   */
  getMetrics: () => TurboManager.getInstance().getPerformanceMetrics(),
  
  /**
   * Execute operation with turbo optimizations
   */
  execute: <T>(operation: () => Promise<T> | T) => 
    TurboManager.getInstance().executeWithTurbo(operation),
  
  /**
   * Create turbo-optimized timeout
   */
  timeout: (callback: () => void, timeout: number) =>
    TurboManager.getInstance().createTimeout(callback, timeout),
  
  /**
   * Create turbo-optimized interval
   */
  interval: (callback: () => void, interval: number) =>
    TurboManager.getInstance().createInterval(callback, interval),
  
  /**
   * Batch operations with turbo optimizations
   */
  batch: <T>(operations: Array<() => Promise<T> | T>, batchSize?: number) =>
    TurboManager.getInstance().batchOperations(operations, batchSize),
};

// Default export
export default turbo;