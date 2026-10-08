// Test runner scope: Micro-UIs and the screen composer only (apps/**).
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['apps/**/*.{test,spec}.{ts,tsx}'],
    // Zero test files is a pass on an empty repo; a failing test still fails.
    passWithNoTests: true,
    // Run once and exit, even on a TTY.
    watch: false,
  },
});
