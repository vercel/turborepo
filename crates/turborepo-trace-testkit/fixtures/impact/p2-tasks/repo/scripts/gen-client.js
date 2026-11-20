// Prints the generated client module (CLIENT_NAME comes from tools/sh/env.sh).
process.stdout.write('module.exports = { name: ' + JSON.stringify(process.env.CLIENT_NAME) + ' };\n');
