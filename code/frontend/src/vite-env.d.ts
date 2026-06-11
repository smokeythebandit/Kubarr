/// <reference types="vite/client" />

declare const __VERSION__: string;
declare const __COMMIT_HASH__: string;
declare const __BUILD_TIME__: string;
declare const __CHANNEL__: string;

interface ImportMetaEnv {
  readonly VITE_API_URL: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
