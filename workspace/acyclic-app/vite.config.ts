import { appendFileSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
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

export default defineConfig({
  plugins: [
    react(),
    // The licences of exactly what the bundle holds, for the release's notices.
    license({ thirdParty: { includePrivate: false, output: { file: "dist/licenses.txt" } } }),
    fontNotice(),
    precompress(),
  ],
  build: { sourcemap: false },
});
