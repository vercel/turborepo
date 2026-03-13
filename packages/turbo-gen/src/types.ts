// Self-contained types for Turborepo generators.
//
// These mirror the node-plop types that consumers interact with, inlined here
// so that `@turbo/gen` users get proper type checking without needing
// `node-plop` (or its transitive type deps like `inquirer`) installed.
//
// We can't use tsdown's `dts.resolve` to bundle node-plop's types because
// rolldown-plugin-dts can't resolve named imports from `export =` modules
// (handlebars uses this pattern).

// eslint-disable-next-line @typescript-eslint/no-namespace
export namespace PlopTypes {
  export interface Answers {
    [key: string]: any;
  }

  export interface IncludeDefinitionConfig {
    generators?: boolean;
    helpers?: boolean;
    partials?: boolean;
    actionTypes?: boolean;
  }

  export type IncludeDefinition = boolean | string[] | IncludeDefinitionConfig;

  export interface PlopCfg {
    force: boolean;
    destBasePath: string | undefined;
  }

  export interface ActionConfig {
    type: string;
    force?: boolean;
    data?: object;
    abortOnFail?: boolean;
    skip?: (...args: any[]) => any;
  }

  interface Template {
    template: string;
    templateFile?: never;
  }

  interface TemplateFile {
    template?: never;
    templateFile: string;
  }

  type TemplateStrOrFile = Template | TemplateFile;

  type TransformFn<T> = (
    template: string,
    data: any,
    cfg: T
  ) => string | Promise<string>;

  interface AddActionConfigBase extends ActionConfig {
    type: "add";
    path: string;
    skipIfExists?: boolean;
    transform?: TransformFn<AddActionConfig>;
  }

  export type AddActionConfig = AddActionConfigBase & TemplateStrOrFile;

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
    globOptions: Record<string, unknown>;
    verbose?: boolean;
    transform?: TransformFn<AddManyActionConfig>;
  }

  interface ModifyActionConfigBase extends ActionConfig {
    type: "modify";
    path: string;
    pattern: string | RegExp;
    transform?: TransformFn<ModifyActionConfig>;
  }

  export type ModifyActionConfig = ModifyActionConfigBase & TemplateStrOrFile;

  interface AppendActionConfigBase extends ActionConfig {
    type: "append";
    path: string;
    pattern: string | RegExp;
    unique: boolean;
    separator: string;
  }

  export type AppendActionConfig = AppendActionConfigBase & TemplateStrOrFile;

  export interface CustomActionConfig<TypeString extends string> extends Omit<
    ActionConfig,
    "type"
  > {
    type: TypeString extends "addMany" | "modify" | "append"
      ? never
      : TypeString;
    [key: string]: any;
  }

  export type CustomActionFunction = (
    answers: Answers,
    config: CustomActionConfig<string>,
    plopfileApi: NodePlopAPI
  ) => Promise<string> | string;

  export type ActionType =
    | string
    | ActionConfig
    | AddActionConfig
    | AddManyActionConfig
    | ModifyActionConfig
    | AppendActionConfig
    | CustomActionFunction;

  export interface PromptQuestion {
    type?: string;
    name?: string;
    message?: string | ((...args: any[]) => string);
    default?: any;
    choices?: Array<any> | ((...args: any[]) => any);
    validate?: (...args: any[]) => boolean | string | Promise<boolean | string>;
    filter?: (...args: any[]) => any;
    transformer?: (...args: any[]) => any;
    when?: boolean | ((...args: any[]) => boolean | Promise<boolean>);
    prefix?: string;
    suffix?: string;
    [key: string]: any;
  }

  export type DynamicPromptsFunction = (inquirer: unknown) => Promise<Answers>;

  export type DynamicActionsFunction = (data?: Answers) => ActionType[];

  export type Prompts = DynamicPromptsFunction | PromptQuestion[];
  export type Actions = DynamicActionsFunction | ActionType[];

  interface PlopActionHooksFailures {
    type: string;
    path: string;
    error: string;
    message: string;
  }

  interface PlopActionHooksChanges {
    type: string;
    path: string;
  }

  interface PlopActionHooks {
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
      answers: Answers,
      hooks?: PlopActionHooks
    ) => Promise<{
      changes: PlopActionHooksChanges[];
      failures: PlopActionHooksFailures[];
    }>;
  }

  export interface NodePlopAPI {
    setGenerator(
      name: string,
      config: Partial<PlopGeneratorConfig>
    ): PlopGenerator;

    setPrompt(name: string, prompt: unknown): void;

    setWelcomeMessage(message: string): void;

    getWelcomeMessage(): string;

    getGenerator(name: string): PlopGenerator;

    getGeneratorList(): { name: string; description: string }[];

    setPartial(name: string, str: string): void;

    getPartial(name: string): string;

    getPartialList(): string[];

    setHelper(name: string, fn: (...args: any[]) => any): void;

    getHelper(name: string): (...args: any[]) => any;

    getHelperList(): string[];

    setActionType(name: string, fn: CustomActionFunction): void;

    getActionType(name: string): ActionType;

    getActionTypeList(): string[];

    setPlopfilePath(filePath: string): void;

    getPlopfilePath(): string;

    getDestBasePath(): string;

    load(
      target: string[] | string,
      loadCfg?: Partial<PlopCfg> | null,
      includeOverride?: IncludeDefinition
    ): Promise<void>;

    setDefaultInclude(inc: object): void;

    getDefaultInclude(): object;

    renderString(template: string, data: any): string;

    /** @deprecated Use "setPrompt" instead. */
    addPrompt(name: string, prompt: unknown): void;

    /** @deprecated Use "setPartial" instead. */
    addPartial(name: string, str: string): void;

    /** @deprecated Use "setHelper" instead. */
    addHelper(name: string, fn: (...args: any[]) => any): void;
  }
}
