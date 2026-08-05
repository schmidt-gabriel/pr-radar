import { existsSync, createReadStream } from "node:fs";
import { resolve } from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

// The port is pinned and strict on purpose. Tauri's `devUrl` points at a fixed
// address, so letting Vite silently fall back to the next free port loads
// whatever else happens to be on 5173 into the app window.
const PORT = 5179;

/**
 * Serves the snapshot fixture during `npm run dev` only.
 *
 * It deliberately does not live in `public/`: everything there is copied into
 * the production bundle, and this file is a dump of real PR data that has no
 * business shipping inside the app.
 *
 * Refresh it with:
 *   cd src-tauri && cargo run --example snapshot -- --json > ../dev/snapshot.json
 */
function devFixture(): Plugin {
  return {
    name: "pr-radar-dev-fixture",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/dev-snapshot.json", (_req, res) => {
        const file = resolve(import.meta.dirname, "dev/snapshot.json");
        if (!existsSync(file)) {
          res.statusCode = 404;
          res.end("{}");
          return;
        }
        res.setHeader("content-type", "application/json");
        createReadStream(file).pipe(res);
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), devFixture()],
  clearScreen: false,
  server: {
    port: PORT,
    strictPort: true,
  },
});
