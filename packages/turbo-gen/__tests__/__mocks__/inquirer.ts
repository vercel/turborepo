// Mock for inquirer (ESM-only in v9, can't be loaded by Jest in CJS mode)
class Separator {
  line: string;
  type: string;
  constructor(line?: string) {
    this.line = line || "--------";
    this.type = "separator";
  }
}

const inquirer = {
  prompt: jest.fn().mockResolvedValue({}),
  registerPrompt: jest.fn(),
  restoreDefaultPrompts: jest.fn(),
  Separator,
  createPromptModule: jest.fn()
};

export default inquirer;
