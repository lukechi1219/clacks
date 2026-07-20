import { defineConfig } from "vite";

// index.html lives in src/ (per architecture.md's flat frontend layout), but
// Vite's default project root is the directory containing vite.config.ts
// (repo root here). Point root at src/ and send build output back to a
// repo-root dist/ so it doesn't land inside src/.
export default defineConfig({
  root: "src",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
  },
});
