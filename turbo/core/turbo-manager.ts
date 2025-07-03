import { TurboConfig, TurboStateCallback, TurboEvents, PerformanceMetrics, OptimizationOptions } from '../types/turbo-types';
import { TurboConfigManager } from '../config/turbo-config';
import { PerformanceUtils } from '../utils/performance-utils';

/**
 * Simple event emitter for turbo events
 */
class SimpleEventEmitter {
  private listeners: Map<string, Array<(data: any) => void>> = new Map();

  on(event: string, listener: (data: any) => void): this {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, []);
    }
    this.listeners.get(event)!.push(listener);
    return this;
  }

  emit(event: string, data: any): boolean {
    const eventListeners = this.listeners.get(event);
    if (eventListeners) {
      eventListeners.forEach(listener => {
        try {
          listener(data);
        } catch (error) {
          console.error(`Error in event listener for ${event}:`, error);
        }
      });
      return true;
    }
    return false;
  }

  removeAllListeners(): void {
    this.listeners.clear();
  }
}

/**
 * Main manager class for turbo mode functionality
 */
export class TurboManager extends SimpleEventEmitter {
  private static instance: TurboManager | null = null;
  private isEnabled: boolean = false;
  private configManager: TurboConfigManager;
  private stateChangeCallbacks: TurboStateCallback[] = [];
  private metricsUpdateInterval: ReturnType<typeof setInterval> | null = null;
  private optimizationOptions: OptimizationOptions = {
    optimizeTimers: true,
    enableBatching: true,
    enableMonitoring: true,
  };

  private constructor(initialConfig?: Partial<TurboConfig>) {
    super();
    this.configManager = new TurboConfigManager(initialConfig);
    this.isEnabled = this.configManager.getConfig().defaultEnabled;
    
    this.setupConfigListener();
    // Don't start metrics updates automatically to avoid keeping process alive
    // this.startMetricsUpdates();

    if (this.isEnabled) {
      this.applyTurboOptimizations();
    }
  }

  /**
   * Get the singleton instance of TurboManager
   */
  static getInstance(initialConfig?: Partial<TurboConfig>): TurboManager {
    if (!TurboManager.instance) {
      TurboManager.instance = new TurboManager(initialConfig);
    }
    return TurboManager.instance;
  }

  /**
   * Enable turbo mode
   */
  enable(): void {
    if (this.isEnabled) return;

    this.isEnabled = true;
    const config = this.configManager.getConfig();
    
    this.applyTurboOptimizations();
    
    const eventData = {
      multiplier: config.speedMultiplier,
      timestamp: Date.now(),
    };

    this.emit('turbo:enabled', eventData);
    this.notifyStateChange(true, config.speedMultiplier);

    if (config.debug) {
      console.log(`Turbo mode enabled with ${config.speedMultiplier}x speed multiplier`);
    }
  }

  /**
   * Disable turbo mode
   */
  disable(): void {
    if (!this.isEnabled) return;

    this.isEnabled = false;
    this.removeTurboOptimizations();

    const eventData = {
      timestamp: Date.now(),
    };

    this.emit('turbo:disabled', eventData);
    this.notifyStateChange(false, 1);

    if (this.configManager.getConfig().debug) {
      console.log('Turbo mode disabled');
    }
  }

  /**
   * Toggle turbo mode on/off
   */
  toggle(): boolean {
    if (this.isEnabled) {
      this.disable();
    } else {
      this.enable();
    }
    return this.isEnabled;
  }

  /**
   * Check if turbo mode is currently enabled
   */
  isEnabledState(): boolean {
    return this.isEnabled;
  }

  /**
   * Get current speed multiplier
   */
  getSpeedMultiplier(): number {
    return this.isEnabled ? this.configManager.getConfig().speedMultiplier : 1;
  }

  /**
   * Update turbo configuration
   */
  updateConfig(updates: Partial<TurboConfig>): void {
    this.configManager.updateConfig(updates);
  }

  /**
   * Get current configuration
   */
  getConfig(): TurboConfig {
    return this.configManager.getConfig();
  }

  /**
   * Subscribe to turbo state changes
   */
  onStateChange(callback: TurboStateCallback): () => void {
    this.stateChangeCallbacks.push(callback);
    
    // Return unsubscribe function
    return () => {
      const index = this.stateChangeCallbacks.indexOf(callback);
      if (index > -1) {
        this.stateChangeCallbacks.splice(index, 1);
      }
    };
  }

  /**
   * Get current performance metrics
   */
  getPerformanceMetrics(): PerformanceMetrics {
    return PerformanceUtils.getMetrics();
  }

  /**
   * Reset performance metrics
   */
  resetMetrics(): void {
    PerformanceUtils.resetMetrics();
  }

  /**
   * Update optimization options
   */
  setOptimizationOptions(options: Partial<OptimizationOptions>): void {
    this.optimizationOptions = { ...this.optimizationOptions, ...options };
    
    if (this.isEnabled) {
      this.applyTurboOptimizations();
    }
  }

  /**
   * Execute an operation with turbo optimizations if enabled
   */
  async executeWithTurbo<T>(operation: () => Promise<T> | T): Promise<T> {
    const { result } = await PerformanceUtils.measureOperation(operation, this.isEnabled);
    return result;
  }

  /**
   * Create a turbo-optimized timeout
   */
  createTimeout(callback: () => void, timeout: number): ReturnType<typeof setTimeout> {
    const multiplier = this.getSpeedMultiplier();
    return PerformanceUtils.turboTimeout(callback, timeout, multiplier);
  }

  /**
   * Create a turbo-optimized interval
   */
  createInterval(callback: () => void, interval: number): ReturnType<typeof setInterval> {
    const multiplier = this.getSpeedMultiplier();
    return PerformanceUtils.turboInterval(callback, interval, multiplier);
  }

  /**
   * Batch operations with turbo optimizations
   */
  async batchOperations<T>(
    operations: Array<() => Promise<T> | T>,
    batchSize: number = 10
  ): Promise<T[]> {
    const multiplier = this.getSpeedMultiplier();
    return PerformanceUtils.batchOperations(operations, batchSize, multiplier);
  }

  /**
   * Cleanup resources
   */
  destroy(): void {
    this.disable();
    
    if (this.metricsUpdateInterval) {
      clearInterval(this.metricsUpdateInterval);
      this.metricsUpdateInterval = null;
    }

    this.removeAllListeners();
    this.stateChangeCallbacks = [];
    
    TurboManager.instance = null;
  }

  private setupConfigListener(): void {
    this.configManager.onConfigChange((config) => {
      this.emit('turbo:config-changed', { config });
      
      if (this.isEnabled) {
        this.applyTurboOptimizations();
      }
    });
  }

  private applyTurboOptimizations(): void {
    const multiplier = this.configManager.getConfig().speedMultiplier;
    PerformanceUtils.applyOptimizations(this.optimizationOptions, multiplier);
  }

  private removeTurboOptimizations(): void {
    PerformanceUtils.applyOptimizations(this.optimizationOptions, 1);
  }

  private notifyStateChange(enabled: boolean, multiplier: number): void {
    this.stateChangeCallbacks.forEach(callback => {
      try {
        callback(enabled, multiplier);
      } catch (error) {
        console.error('Error in turbo state change callback:', error);
      }
    });
  }

  /**
   * Start automatic metrics updates (optional)
   * Call this if you want periodic performance updates
   */
  startMetricsUpdates(intervalMs: number = 5000): void {
    if (this.metricsUpdateInterval) {
      clearInterval(this.metricsUpdateInterval);
    }
    
    this.metricsUpdateInterval = setInterval(() => {
      const metrics = PerformanceUtils.getMetrics();
      this.emit('turbo:performance-update', metrics);
    }, intervalMs);
  }

  /**
   * Stop automatic metrics updates
   */
  stopMetricsUpdates(): void {
    if (this.metricsUpdateInterval) {
      clearInterval(this.metricsUpdateInterval);
      this.metricsUpdateInterval = null;
    }
  }
  // Type-safe event methods
  on<K extends keyof TurboEvents>(event: K, listener: (data: TurboEvents[K]) => void): this {
    return super.on(event as string, listener);
  }

  emit<K extends keyof TurboEvents>(event: K, data: TurboEvents[K]): boolean {
    return super.emit(event as string, data);
  }
}