# Turbo Mode System

A comprehensive turbo mode system that speeds up application operations by a configurable multiplier (default 2x).

## Features

- 🚀 **Toggle turbo mode on/off** - Enable or disable turbo mode with a simple API
- ⚡ **Configurable speed multipliers** - Adjust speed from 1x to 5x (default 2x)
- 📊 **Performance monitoring and metrics** - Track operations per second, response times, and improvements
- 🎯 **Event-driven architecture** - Subscribe to turbo state changes and performance updates
- 🛠️ **Utility functions** - Helper functions for common speed optimizations
- 🔒 **TypeScript support** - Full type safety with comprehensive TypeScript definitions
- 🎛️ **Extensible configuration** - Easily customize turbo behavior for your needs

## Installation

```bash
npm install turbo-mode-system
```

## Quick Start

```typescript
import { turbo, TurboManager } from 'turbo-mode-system';

// Enable turbo mode with default 2x speed
turbo.enable();

// Check if turbo is enabled
console.log(turbo.isEnabled()); // true

// Get current speed multiplier
console.log(turbo.getSpeedMultiplier()); // 2

// Execute operations with turbo optimizations
const result = await turbo.execute(async () => {
  // Your async operation here
  return await fetchData();
});

// Create turbo-optimized timeouts
turbo.timeout(() => {
  console.log('This will run faster in turbo mode!');
}, 1000); // Will run in ~500ms when turbo is 2x

// Disable turbo mode
turbo.disable();
```

## Advanced Usage

### Custom Configuration

```typescript
import { createTurboManager } from 'turbo-mode-system';

const turboManager = createTurboManager({
  speedMultiplier: 3.0,      // 3x speed
  defaultEnabled: true,       // Start with turbo enabled
  maxSpeedMultiplier: 5.0,   // Allow up to 5x speed
  debug: true                 // Enable debug logging
});

// Update configuration at runtime
turboManager.updateConfig({
  speedMultiplier: 2.5,
  debug: false
});
```

### Event Handling

```typescript
import { TurboManager } from 'turbo-mode-system';

const turboManager = TurboManager.getInstance();

// Listen for turbo state changes
turboManager.on('turbo:enabled', (data) => {
  console.log(`Turbo enabled with ${data.multiplier}x speed at ${data.timestamp}`);
});

turboManager.on('turbo:disabled', (data) => {
  console.log(`Turbo disabled at ${data.timestamp}`);
});

turboManager.on('turbo:performance-update', (metrics) => {
  console.log('Performance metrics:', metrics);
});

// Subscribe to state changes with callback
const unsubscribe = turboManager.onStateChange((enabled, multiplier) => {
  console.log(`Turbo ${enabled ? 'enabled' : 'disabled'} - ${multiplier}x speed`);
});

// Unsubscribe when done
unsubscribe();
```

### Performance Monitoring

```typescript
import { TurboManager, PerformanceUtils } from 'turbo-mode-system';

const turboManager = TurboManager.getInstance();

// Get current performance metrics
const metrics = turboManager.getPerformanceMetrics();
console.log('Operations per second:', metrics.operationsPerSecond);
console.log('Average response time:', metrics.averageResponseTime);
console.log('Turbo operations count:', metrics.turboOperationsCount);

// Measure operation performance
const { result, duration } = await PerformanceUtils.measureOperation(
  async () => {
    return await heavyOperation();
  },
  true // isTurboMode
);

console.log(`Operation completed in ${duration}ms`);
```

### Batch Operations

```typescript
import { turbo } from 'turbo-mode-system';

// Batch multiple operations for better performance
const operations = [
  () => fetchUser(1),
  () => fetchUser(2),
  () => fetchUser(3),
  // ... more operations
];

const results = await turbo.batch(operations, 5); // Process 5 at a time
console.log('All operations completed:', results);
```

### Optimization Options

```typescript
import { TurboManager } from 'turbo-mode-system';

const turboManager = TurboManager.getInstance();

// Configure optimization options
turboManager.setOptimizationOptions({
  optimizeTimers: true,      // Optimize setTimeout/setInterval
  enableBatching: true,      // Enable operation batching
  enableMonitoring: true     // Enable performance monitoring
});
```

## API Reference

### TurboManager

The main class for managing turbo mode functionality.

#### Methods

- `enable()` - Enable turbo mode
- `disable()` - Disable turbo mode
- `toggle()` - Toggle turbo mode on/off
- `isEnabledState()` - Check if turbo mode is enabled
- `getSpeedMultiplier()` - Get current speed multiplier
- `updateConfig(updates)` - Update configuration
- `getConfig()` - Get current configuration
- `onStateChange(callback)` - Subscribe to state changes
- `getPerformanceMetrics()` - Get performance metrics
- `resetMetrics()` - Reset performance metrics
- `executeWithTurbo(operation)` - Execute operation with turbo optimizations
- `createTimeout(callback, timeout)` - Create turbo-optimized timeout
- `createInterval(callback, interval)` - Create turbo-optimized interval
- `batchOperations(operations, batchSize)` - Batch operations with turbo optimizations
- `destroy()` - Cleanup resources

#### Events

- `turbo:enabled` - Fired when turbo mode is enabled
- `turbo:disabled` - Fired when turbo mode is disabled
- `turbo:config-changed` - Fired when configuration changes
- `turbo:performance-update` - Fired with performance metrics updates

### PerformanceUtils

Utility class for performance optimizations and measurements.

#### Static Methods

- `adjustTimeout(timeout, multiplier)` - Adjust timeout based on multiplier
- `adjustInterval(interval, multiplier)` - Adjust interval based on multiplier
- `turboTimeout(callback, timeout, multiplier)` - Create turbo-optimized timeout
- `turboInterval(callback, interval, multiplier)` - Create turbo-optimized interval
- `measureOperation(operation, isTurboMode)` - Measure operation execution time
- `getMetrics()` - Get current performance metrics
- `resetMetrics()` - Reset performance metrics
- `batchOperations(operations, batchSize, multiplier)` - Batch operations for performance

### Configuration Types

```typescript
interface TurboConfig {
  speedMultiplier: number;      // Speed multiplier (default: 2.0)
  defaultEnabled: boolean;      // Start enabled (default: false)
  maxSpeedMultiplier: number;   // Maximum multiplier (default: 5.0)
  debug: boolean;               // Enable debug logging (default: false)
}

interface PerformanceMetrics {
  operationsPerSecond: number;
  averageResponseTime: number;
  speedImprovement: number;
  totalTurboTime: number;
  turboOperationsCount: number;
}

interface OptimizationOptions {
  optimizeTimers: boolean;      // Optimize timeouts/intervals
  enableBatching: boolean;      // Enable operation batching
  enableMonitoring: boolean;    // Enable performance monitoring
}
```

## Best Practices

1. **Start with default settings** - The 2x speed multiplier works well for most applications
2. **Monitor performance** - Use the built-in metrics to measure actual improvements
3. **Test thoroughly** - Ensure your application works correctly at higher speeds
4. **Use events** - Subscribe to turbo events to update your UI or logging
5. **Batch operations** - Use the batching utilities for better performance with multiple operations
6. **Handle errors gracefully** - Turbo mode should degrade gracefully when operations fail

## Examples

### Web Application Integration

```typescript
import { turbo } from 'turbo-mode-system';

class WebApp {
  constructor() {
    // Listen for turbo state changes to update UI
    turbo.getInstance().onStateChange((enabled, multiplier) => {
      this.updateTurboUI(enabled, multiplier);
    });
  }

  async loadData() {
    // Use turbo-optimized data loading
    return await turbo.execute(async () => {
      const [users, posts, comments] = await Promise.all([
        this.fetchUsers(),
        this.fetchPosts(),
        this.fetchComments()
      ]);
      return { users, posts, comments };
    });
  }

  startPeriodicUpdates() {
    // Use turbo-optimized intervals
    turbo.interval(() => {
      this.refreshData();
    }, 30000); // Will refresh faster in turbo mode
  }

  updateTurboUI(enabled, multiplier) {
    const indicator = document.getElementById('turbo-indicator');
    indicator.textContent = enabled ? `🚀 ${multiplier}x` : '⏺️ Normal';
    indicator.className = enabled ? 'turbo-active' : 'turbo-inactive';
  }
}
```

### API Request Optimization

```typescript
import { turbo, PerformanceUtils } from 'turbo-mode-system';

class ApiClient {
  async batchRequests(endpoints) {
    const operations = endpoints.map(endpoint => 
      () => this.makeRequest(endpoint)
    );

    // Batch API requests with turbo optimizations
    return await turbo.batch(operations, 3); // 3 concurrent requests
  }

  async makeRequest(endpoint) {
    return await turbo.execute(async () => {
      const response = await fetch(endpoint);
      return await response.json();
    });
  }

  async optimizedPolling(endpoint, interval = 5000) {
    const poll = () => {
      turbo.timeout(async () => {
        try {
          await this.makeRequest(endpoint);
          poll(); // Schedule next poll
        } catch (error) {
          console.error('Polling failed:', error);
          // Retry with exponential backoff
          turbo.timeout(poll, interval * 2);
        }
      }, interval);
    };

    poll();
  }
}
```

## Contributing

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Support

- 📧 Email: support@turbo-mode-system.com
- 🐛 Issues: [GitHub Issues](https://github.com/turbo-mode-system/issues)
- 📖 Documentation: [Full Documentation](https://docs.turbo-mode-system.com)

---

**Turbo Mode System** - Making your applications faster, one operation at a time! 🚀