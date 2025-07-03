/**
 * Basic Usage Examples for Turbo Mode System
 * 
 * This file demonstrates various ways to use the turbo mode system
 * in your applications.
 */

import { turbo, TurboManager, PerformanceUtils } from '../index';

// Example 1: Basic turbo mode operations
async function basicExample() {
  console.log('=== Basic Turbo Mode Example ===');
  
  // Enable turbo mode (2x speed by default)
  turbo.enable();
  console.log('Turbo enabled:', turbo.isEnabled());
  console.log('Speed multiplier:', turbo.getSpeedMultiplier());
  
  // Execute operations with turbo optimizations
  const result = await turbo.execute(async () => {
    // Simulate an async operation
    await new Promise(resolve => setTimeout(resolve, 100));
    return 'Operation completed!';
  });
  
  console.log('Result:', result);
  
  // Create turbo-optimized timeouts
  turbo.timeout(() => {
    console.log('Turbo timeout executed! (faster than normal)');
  }, 1000);
  
  // Toggle turbo mode
  const isEnabled = turbo.toggle();
  console.log('Turbo toggled, now enabled:', isEnabled);
  
  // Disable turbo mode
  turbo.disable();
  console.log('Turbo disabled:', turbo.isEnabled());
}

// Example 2: Advanced configuration
async function advancedConfigExample() {
  console.log('\n=== Advanced Configuration Example ===');
  
  const turboManager = TurboManager.getInstance({
    speedMultiplier: 3.0,      // 3x speed
    defaultEnabled: true,       // Start enabled
    maxSpeedMultiplier: 5.0,   // Allow up to 5x
    debug: true                 // Enable debug logging
  });
  
  console.log('Initial config:', turboManager.getConfig());
  
  // Update configuration at runtime
  turboManager.updateConfig({
    speedMultiplier: 2.5,
    debug: false
  });
  
  console.log('Updated config:', turboManager.getConfig());
}

// Example 3: Event handling
async function eventHandlingExample() {
  console.log('\n=== Event Handling Example ===');
  
  const turboManager = TurboManager.getInstance();
  
  // Listen for turbo events
  turboManager.on('turbo:enabled', (data) => {
    console.log(`🚀 Turbo enabled with ${data.multiplier}x speed at ${new Date(data.timestamp)}`);
  });
  
  turboManager.on('turbo:disabled', (data) => {
    console.log(`⏹️ Turbo disabled at ${new Date(data.timestamp)}`);
  });
  
  turboManager.on('turbo:performance-update', (metrics) => {
    console.log('📊 Performance update:', {
      opsPerSecond: metrics.operationsPerSecond.toFixed(2),
      avgResponseTime: metrics.averageResponseTime.toFixed(2),
      turboOps: metrics.turboOperationsCount
    });
  });
  
  // Subscribe to state changes
  const unsubscribe = turboManager.onStateChange((enabled, multiplier) => {
    console.log(`State changed: Turbo ${enabled ? 'ON' : 'OFF'} (${multiplier}x)`);
  });
  
  // Test events
  turboManager.enable();
  await new Promise(resolve => setTimeout(resolve, 1000));
  turboManager.disable();
  
  // Clean up
  unsubscribe();
}

// Example 4: Performance monitoring
async function performanceExample() {
  console.log('\n=== Performance Monitoring Example ===');
  
  const turboManager = TurboManager.getInstance();
  turboManager.enable();
  
  // Simulate multiple operations
  const operations = Array.from({ length: 10 }, (_, i) => async () => {
    await new Promise(resolve => setTimeout(resolve, Math.random() * 100));
    return `Operation ${i + 1} completed`;
  });
  
  // Execute operations and measure performance
  console.log('Executing operations...');
  for (const operation of operations) {
    await turboManager.executeWithTurbo(operation);
  }
  
  // Get performance metrics
  const metrics = turboManager.getPerformanceMetrics();
  console.log('Performance metrics:', {
    operationsPerSecond: metrics.operationsPerSecond.toFixed(2),
    averageResponseTime: `${metrics.averageResponseTime.toFixed(2)}ms`,
    turboOperationsCount: metrics.turboOperationsCount,
    totalTurboTime: `${metrics.totalTurboTime.toFixed(2)}ms`
  });
  
  turboManager.disable();
}

// Example 5: Batch operations
async function batchOperationsExample() {
  console.log('\n=== Batch Operations Example ===');
  
  turbo.enable();
  
  // Create a batch of operations
  const batchOperations = Array.from({ length: 15 }, (_, i) => 
    async () => {
      // Simulate API calls or other async operations
      await new Promise(resolve => setTimeout(resolve, 50 + Math.random() * 100));
      return `Batch item ${i + 1} processed`;
    }
  );
  
  console.log('Processing batch operations...');
  const startTime = performance.now();
  
  // Process operations in batches of 5
  const results = await turbo.batch(batchOperations, 5);
  
  const endTime = performance.now();
  console.log(`Batch completed in ${(endTime - startTime).toFixed(2)}ms`);
  console.log(`Processed ${results.length} items`);
  
  turbo.disable();
}

// Example 6: Real-world simulation - Data fetching
async function dataFetchingExample() {
  console.log('\n=== Data Fetching Simulation ===');
  
  const turboManager = TurboManager.getInstance();
  
  // Simulate API endpoints
  const simulateApiCall = async (endpoint: string, delay: number = 100) => {
    await new Promise(resolve => setTimeout(resolve, delay));
    return { endpoint, data: `Data from ${endpoint}`, timestamp: Date.now() };
  };
  
  const endpoints = [
    '/api/users',
    '/api/posts',
    '/api/comments',
    '/api/settings',
    '/api/notifications'
  ];
  
  // Test without turbo
  console.log('Fetching data WITHOUT turbo...');
  const normalStart = performance.now();
  
  const normalResults = await Promise.all(
    endpoints.map(endpoint => simulateApiCall(endpoint, 200))
  );
  
  const normalTime = performance.now() - normalStart;
  console.log(`Normal fetch completed in ${normalTime.toFixed(2)}ms`);
  
  // Test with turbo
  console.log('Fetching data WITH turbo...');
  turboManager.enable();
  
  const turboStart = performance.now();
  
  const turboResults = await Promise.all(
    endpoints.map(endpoint => 
      turboManager.executeWithTurbo(() => simulateApiCall(endpoint, 200))
    )
  );
  
  const turboTime = performance.now() - turboStart;
  console.log(`Turbo fetch completed in ${turboTime.toFixed(2)}ms`);
  
  const improvement = ((normalTime - turboTime) / normalTime * 100);
  console.log(`Performance improvement: ${improvement.toFixed(1)}%`);
  
  turboManager.disable();
}

// Example 7: Timer optimizations
async function timerOptimizationsExample() {
  console.log('\n=== Timer Optimizations Example ===');
  
  turbo.enable();
  
  console.log('Starting timers...');
  
  // Regular timeout vs turbo timeout comparison
  const startTime = Date.now();
  
  // This will execute faster due to turbo mode
  turbo.timeout(() => {
    const elapsed = Date.now() - startTime;
    console.log(`Turbo timeout executed after ${elapsed}ms (requested 1000ms)`);
  }, 1000);
  
  // Set up a turbo interval
  let counter = 0;
  const intervalId = turbo.interval(() => {
    counter++;
    console.log(`Turbo interval tick ${counter}`);
    
    if (counter >= 3) {
      clearInterval(intervalId);
      console.log('Turbo interval stopped');
      turbo.disable();
    }
  }, 500);
}

// Run all examples
async function runAllExamples() {
  try {
    await basicExample();
    await advancedConfigExample();
    await eventHandlingExample();
    await performanceExample();
    await batchOperationsExample();
    await dataFetchingExample();
    await timerOptimizationsExample();
    
    console.log('\n✅ All examples completed successfully!');
  } catch (error) {
    console.error('❌ Error running examples:', error);
  }
}

// Export for use in other files
export {
  basicExample,
  advancedConfigExample,
  eventHandlingExample,
  performanceExample,
  batchOperationsExample,
  dataFetchingExample,
  timerOptimizationsExample,
  runAllExamples
};

// To run all examples, import and call runAllExamples() from your application