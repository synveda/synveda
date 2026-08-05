import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The console is served by the gateway under /console/ (ADR-0056
// decision 1), so every emitted asset URL has to be written for that
// prefix. `base` is the whole of what makes same-origin serving work: a
// bundle built for `/` would request /assets/index-*.js, which is not a
// path the gateway serves and not a path ServeDir would find.
export default defineConfig({
  base: "/console/",
  plugins: [react()],
  build: {
    outDir: "dist",
    // A build that quietly reuses stale output is a build that can serve
    // a file the current source cannot produce.
    emptyOutDir: true,
    // The Content-Security-Policy the gateway sets has no 'unsafe-inline'
    // (crates/synveda-gateway/src/console.rs), so nothing may be inlined
    // into the HTML — not the smallest stylesheet and not a data: script.
    // Vite inlines assets under this threshold by default; zero turns that
    // off, so the CSP and the bundle cannot disagree.
    assetsInlineLimit: 0,
  },
});
