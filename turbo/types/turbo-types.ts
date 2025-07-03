/**
 * Configuration interface for turbo mode settings
 */
export interface TurboConfig {
  /** Speed multiplier when turbo mode is enabled (default: 2) */
  speedMultiplier: number;
  /** Whether turbo mode is enabled by default */
  defaultEnabled: boolean;
  /** Maximum allowed speed multiplier */
  maxSpeedMultiplier: number;
  /** Debug mode for performance logging */
  debug: boolean;
}

/**
 * Event types for turbo mode state changes
 */
export interface TurboEvents {
  'turbo:enabled': { multiplier: number; timestamp: number };
  'turbo:disabled': { timestamp: number };
  'turbo:config-changed': { config: TurboConfig };
  'turbo:performance-update': PerformanceMetrics;
}

/**
 * Performance metrics for monitoring turbo mode effectiveness
 */
export interface PerformanceMetrics {
  /** Current operations per second */
  operationsPerSecond: number;
  /** Average response time in milliseconds */
  averageResponseTime: number;
  /** Speed improvement factor */
  speedImprovement: number;
  /** Total turbo time active in milliseconds */
  totalTurboTime: number;
  /** Number of operations completed in turbo mode */
  turboOperationsCount: number;
}

/**
 * Callback function type for turbo state changes
 */
export type TurboStateCallback = (enabled: boolean, multiplier: number) => void;

/**
 * Performance optimization options
 */
export interface OptimizationOptions {
  /** Whether to optimize timeouts and intervals */
  optimizeTimers: boolean;
  /** Whether to batch operations when possible */
  enableBatching: boolean;
  /** Whether to use performance monitoring */
  enableMonitoring: boolean;
}