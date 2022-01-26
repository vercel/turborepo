export interface DocsThemeConfig {
  docsRepositoryBase?: string;
  titleSuffix?:
    | string
    | React.FC<{
        locale: string;
        config: DocsThemeConfig;
        title: string;
        meta: Record<string, any>;
      }>;
  nextLinks?: boolean;
  prevLinks?: boolean;
  search?: boolean;
  darkMode?: boolean;
  defaultMenuCollapsed?: boolean;
  font?: boolean;
  footer?: boolean;
  footerText?: string;
  footerEditLink?: string;
  feedbackLink?: string;
  feedbackLabels?: string;
  head?:
    | React.ReactNode
    | React.FC<{
        locale: string;
        config: DocsThemeConfig;
        title: string;
        meta: Record<string, any>;
      }>;
  logo?: React.ReactNode;
  banner?: React.ReactNode;
  direction?: string;
  i18n?: { locale: string; text: string; direction: string }[];
  customSearch?: boolean;
  searchPlaceholder?: string | ((props: { locale?: string }) => string);
  projectLink?: string;
  github?: string;
  projectLinkIcon?: React.FC<{ locale: string }>;
  projectChatLink?: string;
  enterpriseLink?: string;
  projectChatLinkIcon?: React.FC<{ locale: string }>;
  floatTOC?: boolean;
  unstable_faviconGlyph?: string;
  unstable_flexsearch?: boolean;
  unstable_searchResultEmpty?:
    | React.ReactNode
    | React.FC<{
        locale: string;
        config: DocsThemeConfig;
        title: string;
        meta: Record<string, any>;
      }>;
}
