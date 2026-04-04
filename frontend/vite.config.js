import { defineConfig, loadEnv } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const visorPort = env.VITE_VISOR_PORT || "3847";

  return {
    plugins: [vue()],
    server: {
      proxy: {
        // Dev: WebSocket moet naar de meshcorebot-visor (zelfde pad als productie).
        "/ws": {
          target: `http://127.0.0.1:${visorPort}`,
          ws: true,
          changeOrigin: true,
        },
      },
    },
    build: {
      outDir: "dist",
      emptyOutDir: true,
    },
  };
});
