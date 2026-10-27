# Third-Party Notices — Marguerite

This file is the repository-level entry point for third-party attribution.
It covers the Rust engine/CLI/TUI, the Tauri shell, and the Vue web console
(`webui/`). The Python sidecar keeps its own complete inventory in
[`pyengine/THIRD_PARTY_NOTICES.md`](pyengine/THIRD_PARTY_NOTICES.md); that file
has equal authority for everything under `pyengine/`, including its bundled
skills, fonts, model bundle, and generated Control UI assets.

## 1. Project license

Marguerite itself is distributed under the MIT License. The verbatim header
from [`LICENSE`](LICENSE):

```text
MIT License

Copyright (c) 2026 Marguerite contributors
```

The full text follows in §4 and applies to this project's own source.

## 2. Web console (`webui/`) direct dependencies

Source of truth: `webui/package.json` (`dependencies`). License identifiers
were read from each installed package's `package.json` (`license` field), not
from memory. Version ranges are the declared ranges; resolved versions are
pinned in `webui/package-lock.json`.

| Package | Version range | License |
|---|---|---|
| `@codemirror/autocomplete` | `^6.20.4` | MIT |
| `@codemirror/commands` | `^6.11.1` | MIT |
| `@codemirror/lang-css` | `^6.3.1` | MIT |
| `@codemirror/lang-html` | `^6.4.12` | MIT |
| `@codemirror/lang-javascript` | `^6.2.5` | MIT |
| `@codemirror/lang-json` | `^6.0.2` | MIT |
| `@codemirror/lang-markdown` | `^6.5.2` | MIT |
| `@codemirror/lang-python` | `^6.2.1` | MIT |
| `@codemirror/lang-rust` | `^6.0.2` | MIT |
| `@codemirror/lang-yaml` | `^6.1.3` | MIT |
| `@codemirror/language` | `^6.13.1` | MIT |
| `@codemirror/search` | `^6.7.2` | MIT |
| `@codemirror/state` | `^6.7.6` | MIT |
| `@codemirror/view` | `^6.43.15` | MIT |
| `@lezer/highlight` | `^1.2.5` | MIT |
| `@types/dompurify` | `^3.0.5` | MIT |
| `@types/three` | `^0.185.4` | MIT |
| `dompurify` | `^3.4.12` | MPL-2.0 OR Apache-2.0 |
| `highlight.js` | `^11.11.1` | BSD-3-Clause |
| `html-to-image` | `^1.11.13` | MIT |
| `katex` | `0.19.0` | MIT |
| `marked` | `^18.0.4` | MIT |
| `pinia` | `^2.1.7` | MIT |
| `three` | `^0.185.1` | MIT |
| `vue` | `^3.5.0` | MIT |
| `vue-i18n` | `^11.4.6` | MIT |
| `vue-router` | `^4.3.0` | MIT |

`@types/*` packages ship type declarations only and never enter the runtime
bundle.

### Bundled webfonts

The console ships subsetted webfonts as build assets (`webui/src/assets/fonts/`
and `webui/src/themes/*/fonts/`):

| Font family | Files | License |
|---|---|---|
| IBM Plex Sans / IBM Plex Mono | `ibm-plex-*.woff2` | SIL Open Font License 1.1 |
| Space Grotesk | `space-grotesk-*.woff2` | SIL Open Font License 1.1 |
| Fraunces | `fraunces-*.woff2` | SIL Open Font License 1.1 |
| Newsreader | `newsreader-*.woff2` | SIL Open Font License 1.1 |
| KaTeX fonts | `KaTeX_*` (built from the `katex` package) | MIT |

The in-repo font files are binary subsets without accompanying license files;
the license statements above come from the upstream font projects. When
replacing a font, keep its license file with the new asset.

## 3. Build-only dependencies

`webui/devDependencies` (`@playwright/test`, `@vitejs/plugin-vue`, `axe-core`,
`happy-dom`, `typescript`, `vite`, `vitest`, `vue-tsc`) and `gui/tauri`
tooling are used to build and test the product; they are not redistributed in
installation artifacts.

Rust crates are pinned in `Cargo.lock`. Their license metadata is
machine-readable via `cargo metadata`; a consolidated Rust notice file is not
generated yet (tracked as a P3 item in the remediation plan).

## 4. MIT License text

Applies to every package marked MIT above and to Marguerite's own source.
Each third-party package declares its own copyright holder in its distributed
`package.json` / `LICENSE`.

```text
MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## 5. Other licenses

- **highlight.js** — BSD-3-Clause. Copyright (c) 2006, Ivan Sagalaev. The full
  three-clause text is reproduced in
  [`pyengine/THIRD_PARTY_NOTICES.md`](pyengine/THIRD_PARTY_NOTICES.md) under the
  Web UI section.
- **DOMPurify** — MPL-2.0 OR Apache-2.0. Marguerite distributes it under the
  Apache-2.0 option; the upstream notice is reproduced in
  [`pyengine/THIRD_PARTY_NOTICES.md`](pyengine/THIRD_PARTY_NOTICES.md).
- **KaTeX, marked, Vue, Vue Router, Vue I18n** — MIT; upstream copyright
  notices are reproduced in
  [`pyengine/THIRD_PARTY_NOTICES.md`](pyengine/THIRD_PARTY_NOTICES.md).
