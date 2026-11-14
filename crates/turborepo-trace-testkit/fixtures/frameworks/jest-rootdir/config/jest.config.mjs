import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

export default async () => ({
  rootDir: '..',
  roots: ['<rootDir>/src'],
  modulePaths: ['<rootDir>/shared'],
  moduleDirectories: ['node_modules', '<rootDir>/'],
  modulePathIgnorePatterns: ['<rootDir>/src/.*/__fixtures__'],
  testRegex: ['\\.unit\\.[jt]s$'],
  testPathIgnorePatterns: ['<rootDir>/src/skip/'],
  snapshotResolver: path.join(__dirname, 'snapshot-resolver.cjs'),
  moduleFileExtensions: ['ts', 'js'],
  preset: './config/preset.json',
});
