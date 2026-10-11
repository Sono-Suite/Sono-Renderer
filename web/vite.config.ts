import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  base: "/",
  build: {
    sourcemap: false,
    rollupOptions: {
      output: {
        entryFileNames: "admin.js",
        chunkFileNames: "chunk-[name].js",
        assetFileNames: (asset) =>
          asset.name?.endsWith(".css") ? "admin.css" : "[name][extname]",
      },
    },
  },
});
