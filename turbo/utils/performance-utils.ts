import { PerformanceMetrics, OptimizationOptions } from '../types/turbo-types';

/**
 * Utility class for performance optimizations and measurements
 */
export class PerformanceUtils {
  private static metrics: PerformanceMetrics = {
    operationsPerSecond: 0,
    averageResponseTime: 0,
    speedImprovement: 1,
    totalTurboTime: 0,
    turboOperationsCount: 0,
  };

  private static operationTimes: number[] = [];
  private static turboStartTime: number | null = null;

  /**
   * Adjust timeout duration based on turbo multiplier
   */
  static adjustTimeout(originalTimeout: number, speedMultiplier: number): number {
    return Math.max(1, Math.floor(originalTimeout / speedMultiplier));
  }

  /**
   * Adjust interval duration based on turbo multiplier
   */
  static adjustInterval(originalInterval: number, speedMultiplier: number): number {
    return Math.max(1, Math.floor(originalInterval / speedMultiplier));
  }

  /**
   * Create a turbo-optimized setTimeout
   */
  static turboTimeout(
    callback: () => void,
    timeout: number,
    speedMultiplier: number = 1
  ): ReturnType<typeof setTimeout> {
    const adjustedTimeout = this.adjustTimeout(timeout, speedMultiplier);
    return setTimeout(callback, adjustedTimeout);
  }

  /**
   * Create a turbo-optimized setInterval
   */
  static turboInterval(
    callback: () => void,
    interval: number,
    speedMultiplier: number = 1
  ): ReturnType<typeof setInterval> {
    const adjustedInterval = this.adjustInterval(interval, speedMultiplier);
    return setInterval(callback, adjustedInterval);
  }

  /**
   * Measure execution time of an operation
   */
  static async measureOperation<T>(
    operation: () => Promise<T> | T,
    isTurboMode: boolean = false
  ): Promise<{ result: T; duration: number }> {
    const startTime = performance.now();
    
    try {
      const result = await operation();
      const duration = performance.now() - startTime;
      
      this.recordOperationTime(duration, isTurboMode);
      
      return { result, duration };
    } catch (error) {
      const duration = performance.now() - startTime;
      this.recordOperationTime(duration, isTurboMode);
      throw error;
    }
  }

  /**
   * Record operation time for performance metrics
   */
  private static recordOperationTime(duration: number, isTurboMode: boolean): void {
    this.operationTimes.push(duration);
    
    // Keep only the last 100 operations for rolling average
    if (this.operationTimes.length > 100) {
      this.operationTimes.shift();
    }

    if (isTurboMode) {
      this.metrics.turboOperationsCount++;
    }

    this.updateMetrics();
  }

  /**
   * Update performance metrics
   */
  private static updateMetrics(): void {
    if (this.operationTimes.length === 0) return;

    const totalTime = this.operationTimes.reduce((sum, time) => sum + time, 0);
    this.metrics.averageResponseTime = totalTime / this.operationTimes.length;
    this.metrics.operationsPerSecond = 1000 / this.metrics.averageResponseTime;

    if (this.turboStartTime) {
      this.metrics.totalTurboTime = performance.now() - this.turboStartTime;
    }
  }

  /**
   * Start turbo mode timing
   */
  static startTurboTiming(): void {
    this.turboStartTime = performance.now();
  }

  /**
   * Stop turbo mode timing
   */
  static stopTurboTiming(): void {
    if (this.turboStartTime) {
      this.metrics.totalTurboTime += performance.now() - this.turboStartTime;
      this.turboStartTime = null;
    }
  }

  /**
   * Get current performance metrics
   */
  static getMetrics(): PerformanceMetrics {
    return { ...this.metrics };
  }

  /**
   * Reset performance metrics
   */
  static resetMetrics(): void {
    this.metrics = {
      operationsPerSecond: 0,
      averageResponseTime: 0,
      speedImprovement: 1,
      totalTurboTime: 0,
      turboOperationsCount: 0,
    };
    this.operationTimes = [];
    this.turboStartTime = null;
  }

  /**
   * Batch multiple operations for better performance
   */
  static async batchOperations<T>(
    operations: Array<() => Promise<T> | T>,
    batchSize: number = 10,
    speedMultiplier: number = 1
  ): Promise<T[]> {
    const results: T[] = [];
    const delayBetweenBatches = this.adjustTimeout(10, speedMultiplier);

    for (let i = 0; i < operations.length; i += batchSize) {
      const batch = operations.slice(i, i + batchSize);
      const batchResults = await Promise.all(
        batch.map(op => this.measureOperation(op, speedMultiplier > 1))
      );
      
      results.push(...batchResults.map(r => r.result));

      // Small delay between batches to prevent overwhelming the system
      if (i + batchSize < operations.length) {
        await new Promise(resolve => setTimeout(resolve, delayBetweenBatches));
      }
    }

    return results;
  }

  /**
   * Apply optimizations based on options
   */
  static applyOptimizations(options: OptimizationOptions, speedMultiplier: number): void {
    if (options.enableMonitoring && speedMultiplier > 1) {
      this.startTurboTiming();
    } else if (!options.enableMonitoring || speedMultiplier === 1) {
      this.stopTurboTiming();
    }

    // Additional optimizations can be added here based on options
    if (options.optimizeTimers) {
      // Timer optimizations are handled by turboTimeout and turboInterval methods
    }

    if (options.enableBatching) {
      // Batching optimizations are available through batchOperations method
    }
  }
}