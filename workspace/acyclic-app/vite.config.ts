import { appendFileSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import type { ClientRequest } from "node:http";
import { join } from "node:path";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";
import react from "@vitejs/plugin-react";
import license from "rollup-plugin-license";
import { defineConfig, type Plugin } from "vite";

/** The text files worth compressing; images such as PNGs are compressed already. */
const COMPRESSIBLE = /\.(html|js|css|json|webmanifest|svg|txt|map)$/;

/** Writes a Brotli and a gzip copy beside each compressible file, for the server to pick by `Accept-Encoding`. */
function precompress(): Plugin {
  return {
    name: "acyclic-precompress",
    apply: "build",
    closeBundle() {
      const walk = (dir: string): string[] =>
        readdirSync(dir).flatMap((name) => {
          const path = join(dir, name);
          return statSync(path).isDirectory() ? walk(path) : [path];
        });
      for (const path of walk("dist").filter((path) => COMPRESSIBLE.test(path))) {
        const body = readFileSync(path);
        const br = brotliCompressSync(body, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } });
        const gz = gzipSync(body, { level: 9 });
        // A copy that saves nothing would only cost the binary its size.
        if (br.length < body.length) writeFileSync(`${path}.br`, br);
        if (gz.length < body.length) writeFileSync(`${path}.gz`, gz);
      }
    },
  };
}

/** Adds the font's licence to the list: app.css loads its files by URL, which the licence plugin doesn't follow. */
function fontNotice(): Plugin {
  return {
    name: "acyclic-font-notice",
    apply: "build",
    writeBundle() {
      const font = "node_modules/@fontsource-variable/overpass";
      const { version } = JSON.parse(readFileSync(`${font}/package.json`, "utf8")) as { version: string };
      const text = readFileSync(`${font}/LICENSE`, "utf8");
      appendFileSync(
        "dist/licenses.txt",
        `\n\n---\n\nName: @fontsource-variable/overpass\nVersion: ${version}\nLicense: OFL-1.1\nLicense Text:\n===\n\n${text}`,
      );
    },
  };
}

/**
 * The most a JavaScript chunk may weigh, minified, as Vite's own warning has it by default. ELK's worker, which every
 * Map loads, isn't a chunk but a file the app opens by its address (`?url`), so it is outside the budget.
 */
const CHUNK_BUDGET = 500 * 1000;

/**
 * Fails the build for a chunk over the budget, where Vite only warns, which went unnoticed in the build's output. One
 * chunk is let through: ELK's bundled engine, which runs layouts on the page's own thread and loads only for
 * `?engine=inline`, as the timing script compares it with the worker the app uses. Splitting it gains nothing, as it
 * loads whole or not at all, and leaving it out of the build would leave the script nothing to time.
 */
function chunkBudget(): Plugin {
  return {
    name: "acyclic-chunk-budget",
    apply: "build",
    generateBundle(_, bundle) {
      for (const chunk of Object.values(bundle)) {
        if (chunk.type !== "chunk" || chunk.facadeModuleId?.endsWith("elkjs/lib/elk.bundled.js")) continue;
        const size = Buffer.byteLength(chunk.code);
        if (size > CHUNK_BUDGET)
          this.error(
            `${chunk.fileName} is ${Math.round(size / 1000)} kB, over the ${CHUNK_BUDGET / 1000} kB budget: load ` +
              "what it holds lazily, or let it through in vite.config.ts's chunkBudget with the reason beside it",
          );
      }
    },
  };
}

export default defineConfig({
  plugins: [
    react(),
    // The licences of exactly what the bundle holds, for the release's notices.
    license({ thirdParty: { includePrivate: false, output: { file: "dist/licenses.txt" } } }),
    fontNotice(),
    precompress(),
    chunkBudget(),
  ],
  // Vite's own warning can't leave one chunk out, so `chunkBudget` keeps the limit instead.
  build: { sourcemap: false, chunkSizeWarningLimit: Number.POSITIVE_INFINITY },
  server: { proxy: upstream(process.env.ACYCLIC_UP) },
});

/**
 * In development (`mise run app:dev`), passes the API and the pairing link to `acyclic up` at `origin`, as that server's
 * own origin: it refuses a request from any other.
 */
function upstream(origin: string | undefined) {
  if (origin === undefined) return undefined;
  const route = {
    target: origin,
    changeOrigin: true,
    configure: (proxy: { on: (event: "proxyReq", listener: (request: ClientRequest) => void) => void }) =>
      proxy.on("proxyReq", (request) => {
        if (request.hasHeader("origin")) request.setHeader("origin", origin);
      }),
  };
  return { "/api": route, "/login": route };
}
