import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['test/**/*.test.ts'],
    hookTimeout: 60_000,
    testTimeout: 30_000,
    // Single fork: the tests share one throwaway Postgres cluster.
    pool: 'forks',
    poolOptions: { forks: { singleFork: true } },
  },
});
