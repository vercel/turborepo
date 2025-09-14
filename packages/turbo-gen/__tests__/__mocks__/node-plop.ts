import { jest } from "@jest/globals";

// Mock implementation of node-plop for Jest tests
const mockNodePlop = jest.fn(() => ({
  getGeneratorList: jest.fn(() => []),
  getGenerator: jest.fn(() => ({
    runPrompts: jest.fn(() => Promise.resolve([])),
    runActions: jest.fn(() => Promise.resolve({ failures: [], changes: [] })),
  })),
  load: jest.fn(() => Promise.resolve()),
}));

export default mockNodePlop;
