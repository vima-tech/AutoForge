# Asset inventory — autoforge-promo

This is the no-capture path: AutoForge is a local Tauri desktop app with no public URL
to crawl, and no screenshots were supplied.

## Available brand asset

- `.media/images/logo_001.png` — official AutoForge application logo ingested from
  `src/assets/logo.png`; use only in the final brand lockup, never redraw it.

All visuals are LLM-invented per frame (typography, UI-flavored panels, pipeline diagrams,
data-viz) built on the brand tokens in `capture/extracted/tokens.json`
(Ember design system: dark warm ground #16110d, single ember accent #e8772e,
Archivo display / Noto Sans SC body / JetBrains Mono chrome).

If real UI screenshots become available later, drop them into `capture/assets/` and
re-run `stage-assets.mjs` before Step 5.
