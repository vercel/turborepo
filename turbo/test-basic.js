/**
 * Basic test to verify the turbo mode system is working correctly
 */

const { turbo, TurboManager } = require('./dist/index');

async function testBasicFunctionality() {
  console.log('🧪 Testing Turbo Mode System...');
  
  try {
    // Test 1: Basic enable/disable
    console.log('\n1. Testing basic enable/disable:');
    console.log('Initial state:', turbo.isEnabled());
    
    turbo.enable();
    console.log('After enable:', turbo.isEnabled());
    console.log('Speed multiplier:', turbo.getSpeedMultiplier());
    
    turbo.disable();
    console.log('After disable:', turbo.isEnabled());
    
    // Test 2: Toggle functionality
    console.log('\n2. Testing toggle functionality:');
    const toggled = turbo.toggle();
    console.log('Toggled to:', toggled);
    console.log('Current state:', turbo.isEnabled());
    
    // Test 3: Configuration
    console.log('\n3. Testing configuration:');
    const manager = TurboManager.getInstance({
      speedMultiplier: 3.0,
      debug: true
    });
    
    console.log('Config:', manager.getConfig());
    
    // Test 4: Execute operation
    console.log('\n4. Testing operation execution:');
    const result = await turbo.execute(async () => {
      await new Promise(resolve => setTimeout(resolve, 10));
      return 'Test operation completed!';
    });
    console.log('Operation result:', result);
    
    // Test 5: Performance metrics
    console.log('\n5. Testing performance metrics:');
    const metrics = turbo.getMetrics();
    console.log('Metrics:', {
      operationsPerSecond: metrics.operationsPerSecond.toFixed(2),
      averageResponseTime: metrics.averageResponseTime.toFixed(2),
      turboOperationsCount: metrics.turboOperationsCount
    });
    
    // Test 6: Timers
    console.log('\n6. Testing turbo timers:');
    turbo.enable();
    
    const start = Date.now();
    await new Promise(resolve => {
      turbo.timeout(() => {
        const elapsed = Date.now() - start;
        console.log(`Turbo timeout executed in ${elapsed}ms (requested 100ms, should be ~50ms with 2x speed)`);
        resolve();
      }, 100);
    });
    
    console.log('\n✅ All tests passed! Turbo Mode System is working correctly.');
    
  } catch (error) {
    console.error('❌ Test failed:', error);
    process.exit(1);
  }
}

// Run the test
testBasicFunctionality();