exports.render = (header, schema) =>
  header + 'module.exports = ' + JSON.stringify(schema) + ';\n';
