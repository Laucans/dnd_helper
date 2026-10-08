// Lint scope mirrors tsconfig.json: apps/** and the root TS tooling files.
import js from '@eslint/js';
import { defineConfig } from 'eslint/config';
import tseslint from 'typescript-eslint';

export default defineConfig(
  {
    // crates/ is Rust; this file is JS outside the tsc project.
    ignores: ['crates/**', 'target/**', 'dist/**', 'coverage/**', 'eslint.config.js'],
  },
  {
    linterOptions: {
      reportUnusedDisableDirectives: 'error',
      reportUnusedInlineConfigs: 'error',
    },
  },
  {
    // Same extensions as tsconfig.json "include".
    files: ['**/*.{ts,tsx}'],
    extends: [js.configs.recommended, tseslint.configs.strictTypeChecked],
    languageOptions: {
      parserOptions: {
        // Type information from tsconfig.json; a file outside it is an error.
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
);
