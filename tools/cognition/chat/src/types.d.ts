/// <reference types="react" />

// Add global declarations for external libraries
declare global {
  interface Window {
    Prism: {
      highlightAll: () => void;
      highlight: (code: string, grammar: any, language: string) => string;
      languages: Record<string, any>;
    };
  }
}

// Remove file extensions from imports for cleaner syntax
declare module "*.ts" {
  const content: any;
  export default content;
}

// Allow importing utils without extensions
declare module "./utils/helpers" {
  import * as helpers from "./utils/helpers.ts";
  export = helpers;
}

// Library modules declarations
declare module "markdown-it" {
  interface MarkdownIt {
    render: (content: string) => string;
    utils: {
      escapeHtml: (text: string) => string;
    };
  }

  interface Options {
    html?: boolean;
    breaks?: boolean;
    linkify?: boolean;
    highlight?: (code: string, lang: string) => string;
  }

  function MarkdownIt(options?: Options): MarkdownIt;
  export = MarkdownIt;
}

declare module "prismjs" {
  const Prism: {
    highlight: (code: string, grammar: any, language: string) => string;
    languages: Record<string, any>;
    highlightAll: () => void;
  };
  export = Prism;
}

declare module "mermaid" {
  const mermaid: {
    initialize: (config: any) => void;
    init: (config: any, elements: NodeListOf<Element>) => void;
  };
  export = mermaid;
}

// Temporary module declarations for .jsx files until they're migrated
declare module "*.jsx" {
  import React from "react";
  const Component: React.ComponentType<any>;
  export default Component;
}

// Component declarations for Input components
declare module "./components/Input/UtilitiesContainer" {
  import React from "react";
  const UtilitiesContainer: React.FC;
  export default UtilitiesContainer;
}

// Component declarations for Chat components
declare module "./components/Chat/UserMessage" {
  import React from "react";
  const UserMessage: React.FC<{ content: string; index: number }>;
  export default UserMessage;
}

declare module "./components/Chat/AssistantMessage" {
  import React from "react";
  const AssistantMessage: React.FC<{
    content: string;
    context?: any;
    index: number;
  }>;
  export default AssistantMessage;
}

declare module "./components/Chat/SystemMessage" {
  import React from "react";
  const SystemMessage: React.FC<{ content: string }>;
  export default SystemMessage;
}

declare module "./components/Chat/ToolMessage" {
  import React from "react";
  const ToolMessage: React.FC<{ content: string }>;
  export default ToolMessage;
}

declare module "./components/Chat/ErrorMessage" {
  import React from "react";
  const ErrorMessage: React.FC<{ error: string }>;
  export default ErrorMessage;
}

declare module "./components/Chat/ThinkingProcess" {
  import React from "react";
  const ThinkingProcess: React.FC<{ content: string; startTime: Date | null }>;
  export default ThinkingProcess;
}

// Specific module declarations for components
declare module "./components/Sidebar/SourcesSection" {
  import React from "react";
  const SourcesSection: React.FC;
  export default SourcesSection;
}

declare module "./components/Sidebar/ToolsSection" {
  import React from "react";
  const ToolsSection: React.FC;
  export default ToolsSection;
}

// Asset declarations
declare module "*.jpg" {
  const src: string;
  export default src;
}

declare module "*.jpeg" {
  const src: string;
  export default src;
}

declare module "*.png" {
  const src: string;
  export default src;
}

declare module "*.svg" {
  import * as React from "react";
  export const ReactComponent: React.FunctionComponent<
    React.SVGProps<SVGSVGElement> & { title?: string }
  >;
  const src: string;
  export default src;
}

// Module declarations for CSS files
declare module "*.css" {
  const css: { [key: string]: string };
  export default css;
}

declare module "*.module.css" {
  const classes: { readonly [key: string]: string };
  export default classes;
}

// Add ContextSection component declaration
declare module "./components/Chat/ContextSection" {
  import React from "react";

  interface ContextMessage {
    role: string;
    content: string;
    images?: Array<{ data: string; type: string }>;
  }

  const ContextSection: React.FC<{
    context: ContextMessage[] | null | undefined;
  }>;

  export default ContextSection;
}
