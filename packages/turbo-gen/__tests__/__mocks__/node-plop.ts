// Mock for node-plop (ESM-only module that can't be loaded by Jest in CJS mode)
export interface NodePlopAPI {
  setGenerator: jest.Mock;
  getGenerator: jest.Mock;
  getGeneratorList: jest.Mock;
  load: jest.Mock;
  setPlopfilePath: jest.Mock;
  getPlopfilePath: jest.Mock;
  getDestBasePath: jest.Mock;
  renderString: jest.Mock;
  setHelper: jest.Mock;
  setPartial: jest.Mock;
  setActionType: jest.Mock;
  setPrompt: jest.Mock;
  setWelcomeMessage: jest.Mock;
  getWelcomeMessage: jest.Mock;
  getHelper: jest.Mock;
  getHelperList: jest.Mock;
  getPartial: jest.Mock;
  getPartialList: jest.Mock;
  getActionType: jest.Mock;
  getActionTypeList: jest.Mock;
  setDefaultInclude: jest.Mock;
  getDefaultInclude: jest.Mock;
}

export interface PlopGenerator {
  name: string;
  description: string;
  prompts: unknown[];
  actions: unknown[];
  runPrompts: jest.Mock;
  runActions: jest.Mock;
}

const nodePlop = jest.fn().mockResolvedValue({
  setGenerator: jest.fn(),
  getGenerator: jest.fn(),
  getGeneratorList: jest.fn().mockReturnValue([]),
  load: jest.fn().mockResolvedValue(undefined),
  setPlopfilePath: jest.fn(),
  getPlopfilePath: jest.fn(),
  getDestBasePath: jest.fn(),
  renderString: jest.fn(),
  setHelper: jest.fn(),
  setPartial: jest.fn(),
  setActionType: jest.fn(),
  setPrompt: jest.fn(),
  setWelcomeMessage: jest.fn(),
  getWelcomeMessage: jest.fn(),
  getHelper: jest.fn(),
  getHelperList: jest.fn(),
  getPartial: jest.fn(),
  getPartialList: jest.fn(),
  getActionType: jest.fn(),
  getActionTypeList: jest.fn(),
  setDefaultInclude: jest.fn(),
  getDefaultInclude: jest.fn()
} as NodePlopAPI);

export default nodePlop;
