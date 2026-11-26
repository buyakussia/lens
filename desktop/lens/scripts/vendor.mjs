// Копирует браузерные сборки библиотек из node_modules в src/vendor (приложение работает без сборщика).
import { copyFileSync, mkdirSync } from "node:fs";
mkdirSync("src/vendor", { recursive: true });
const files = [
  ["node_modules/marked/lib/marked.umd.js", "src/vendor/marked.js"],
  ["node_modules/dompurify/dist/purify.min.js", "src/vendor/purify.min.js"],
  ["node_modules/@highlightjs/cdn-assets/highlight.min.js", "src/vendor/highlight.min.js"],
  ["node_modules/@highlightjs/cdn-assets/styles/github.min.css", "src/vendor/hljs-light.css"],
  ["node_modules/@highlightjs/cdn-assets/styles/github-dark.min.css", "src/vendor/hljs-dark.css"],
];
for (const [from, to] of files) { copyFileSync(from, to); console.log("✓", to); }
