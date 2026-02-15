/**
 * Self-contained type definitions for node-plop.
 *
 * These types are vendored from node-plop@0.26.3 so that consumers of
 * `@turbo/gen` get full type information without needing `node-plop`
 * installed as a dependency.
 *
 * External type imports (inquirer, globby, handlebars) have been replaced
 * with inline equivalents to keep the published declaration self-contained.
 */

// ---------------------------------------------------------------------------
// Inline replacements for external types
// ---------------------------------------------------------------------------

// Simplified inquirer types – only the subset referenced by node-plop's public
// API surface.  Consumers interact with these through PlopGeneratorConfig's
// `prompts` field, which accepts plain objects describing questions.
interface PromptQuestionBase {
  type?: string;
  name?: string;
  message?: string | ((answers: Answers) => string);
  default?: any;
  choices?: any;
  validate?: (
    input: any,
    answers?: Answers
  ) => boolean | string | Promise<boolean | string>;
  filter?: (input: any, answers?: Answers) => any;
  transformer?: (input: any, answers?: Answers, flags?: any) => any;
  when?: boolean | ((answers: Answers) => boolean | Promise<boolean>);
  prefix?: string;
  suffix?: string;
}

type Answers = Record<string, any>;

// ---------------------------------------------------------------------------
// node-plop type definitions (from node-plop@0.26.3 types/index.d.ts)
// ---------------------------------------------------------------------------

export interface NodePlopAPI {
  getGenerator(name: string): PlopGenerator;
  setGenerator(name: string, config: PlopGeneratorConfig): PlopGenerator;

  setPrompt(name: string, prompt: any): void;
  setWelcomeMessage(message: string): void;
  getWelcomeMessage(): string;
  getGeneratorList(): { name: string; description: string }[];
  setPartial(name: string, str: string): void;
  getPartial(name: string): string;
  getPartialList(): string[];
  setHelper(name: string, fn: (...args: any[]) => any): void;
  getHelper(name: string): Function;
  getHelperList(): string[];
  setActionType(name: string, fn: CustomActionFunction): void;
  getActionType(name: string): ActionType;
  getActionTypeList(): string[];

  setPlopfilePath(filePath: string): void;
  getPlopfilePath(): string;
  getDestBasePath(): string;

  // plop.load functionality
  load(
    target: string[] | string,
    loadCfg?: PlopCfg,
    includeOverride?: boolean
  ): void;
  setDefaultInclude(inc: object): void;
  getDefaultInclude(): object;

  renderString(template: string, data: any): string;

  // passthroughs for backward compatibility
  addPrompt(name: string, prompt: any): void;
  addPartial(name: string, str: string): void;
  addHelper(name: string, fn: Function): void;
}

export interface PlopActionHooksFailures {
  type: string;
  path: string;
  error: string;
  message: string;
}

export interface PlopActionHooksChanges {
  type: string;
  path: string;
}

export interface PlopActionHooks {
  onComment?: (msg: string) => void;
  onSuccess?: (change: PlopActionHooksChanges) => void;
  onFailure?: (failure: PlopActionHooksFailures) => void;
}

export interface PlopGeneratorConfig {
  description: string;
  prompts: Prompts;
  actions: Actions;
}

export interface PlopGenerator extends PlopGeneratorConfig {
  runPrompts: (bypassArr?: string[]) => Promise<any>;
  runActions: (
    answers: any,
    hooks?: PlopActionHooks
  ) => Promise<{
    changes: PlopActionHooksChanges[];
    failures: PlopActionHooksFailures[];
  }>;
}

export type PromptQuestion = PromptQuestionBase;

export type DynamicPromptsFunction = (inquirer: any) => Promise<Answers>;
export type DynamicActionsFunction = (data?: Answers) => ActionType[];

export type Prompts = DynamicPromptsFunction | PromptQuestion[];
export type Actions = DynamicActionsFunction | ActionType[];

export type CustomActionFunction = (
  answers: object,
  config?: ActionConfig,
  plopfileApi?: NodePlopAPI
) => Promise<string> | string;

export type ActionType =
  | string
  | ActionConfig
  | AddActionConfig
  | AddManyActionConfig
  | ModifyActionConfig
  | AppendActionConfig
  | CustomActionFunction;

export interface ActionConfig {
  type: string;
  force?: boolean;
  data?: object;
  abortOnFail?: boolean;
  skip?: Function;
}

type TransformFn<T> = (
  template: string,
  data: any,
  cfg: T
) => string | Promise<string>;

interface AddActionConfigBase extends ActionConfig {
  type: "add";
  path: string;
  skipIfExists?: boolean;
}

interface AddActionConfigWithTemplate extends AddActionConfigBase {
  template: string;
}

interface AddActionConfigWithTemplateFile extends AddActionConfigBase {
  templateFile: string;
}

interface AddActionConfigWithTransform extends AddActionConfigBase {
  transform: TransformFn<AddActionConfig>;
  template?: string;
  templateFile?: string;
}

export type AddActionConfig =
  | AddActionConfigWithTemplate
  | AddActionConfigWithTemplateFile
  | AddActionConfigWithTransform;

export interface AddManyActionConfig extends Pick<
  AddActionConfig,
  Exclude<
    keyof AddActionConfig,
    "type" | "templateFile" | "template" | "transform"
  >
> {
  type: "addMany";
  destination: string;
  base: string;
  templateFiles: string | string[];
  stripExtensions?: string[];
  globOptions: Record<string, any>;
  verbose?: boolean;
  transform?: TransformFn<AddManyActionConfig>;
}

export interface ModifyActionConfig extends ActionConfig {
  type: "modify";
  path: string;
  pattern: string | RegExp;
  template: string;
  templateFile: string;
  transform?: TransformFn<ModifyActionConfig>;
}

export interface AppendActionConfig extends ActionConfig {
  type: "append";
  path: string;
  pattern: string | RegExp;
  unique: boolean;
  separator: string;
  template: string;
  templateFile: string;
}

export interface PlopCfg {
  force: boolean;
  destBasePath: string;
}
