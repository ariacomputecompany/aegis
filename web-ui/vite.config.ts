import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  base: "/web-ui/",
  server: {
    proxy: {
      "/chrome": "http://127.0.0.1:7878",
      "/navigate": "http://127.0.0.1:7878",
    },
  },
});
