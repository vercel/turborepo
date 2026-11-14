// Platform branches: both alternatives are kept.
const impl = process.platform === 'linux' ? require('./impl/linux') : require('./impl/darwin');
console.log(impl.NAME);
