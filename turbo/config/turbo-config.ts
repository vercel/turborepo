import { TurboConfig } from '../types/turbo-types';

/**
 * Default turbo configuration settings
 */
export const DEFAULT_TURBO_CONFIG: TurboConfig = {
  speedMultiplier: 2.0,
  defaultEnabled: false,
  maxSpeedMultiplier: 5.0,
  debug: false,
};

/**
 * Configuration manager for turbo mode settings
 */
export class TurboConfigManager {
  private config: TurboConfig;
  private configChangeCallbacks: Array<(config: TurboConfig) => void> = [];

  constructor(initialConfig?: Partial<TurboConfig>) {
    this.config = { ...DEFAULT_TURBO_CONFIG, ...initialConfig };
  }

  /**
   * Get the current configuration
   */
  getConfig(): TurboConfig {
    return { ...this.config };
  }

  /**
   * Update configuration settings
   */
  updateConfig(updates: Partial<TurboConfig>): void {
    const oldConfig = { ...this.config };
    this.config = { ...this.config, ...updates };

    // Validate speed multiplier bounds
    if (this.config.speedMultiplier > this.config.maxSpeedMultiplier) {
      this.config.speedMultiplier = this.config.maxSpeedMultiplier;
    }
    if (this.config.speedMultiplier < 1) {
      this.config.speedMultiplier = 1;
    }

    // Notify callbacks of config changes
    this.configChangeCallbacks.forEach(callback => {
      try {
        callback(this.config);
      } catch (error) {
        console.error('Error in config change callback:', error);
      }
    });

    if (this.config.debug) {
      console.log('Turbo config updated:', {
        old: oldConfig,
        new: this.config,
        changes: updates,
      });
    }
  }

  /**
   * Subscribe to configuration changes
   */
  onConfigChange(callback: (config: TurboConfig) => void): () => void {
    this.configChangeCallbacks.push(callback);
    
    // Return unsubscribe function
    return () => {
      const index = this.configChangeCallbacks.indexOf(callback);
      if (index > -1) {
        this.configChangeCallbacks.splice(index, 1);
      }
    };
  }

  /**
   * Reset configuration to defaults
   */
  resetToDefaults(): void {
    this.updateConfig(DEFAULT_TURBO_CONFIG);
  }

  /**
   * Validate configuration values
   */
  isValidConfig(config: Partial<TurboConfig>): boolean {
    if (config.speedMultiplier !== undefined) {
      if (config.speedMultiplier < 1 || config.speedMultiplier > 10) {
        return false;
      }
    }
    
    if (config.maxSpeedMultiplier !== undefined) {
      if (config.maxSpeedMultiplier < 1 || config.maxSpeedMultiplier > 10) {
        return false;
      }
    }

    return true;
  }
}