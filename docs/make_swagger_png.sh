#!/usr/bin/env bash
#
# make_swagger_png.sh — régénère docs/swagger.png à partir de la spec OpenAPI
# courante de peppol_api (openapi_spec()). À relancer quand l'API change.
#
# Rend Swagger UI HORS-LIGNE (assets swagger-ui-dist récupérés via npm, spec
# inline) puis capture avec Chromium (Playwright). Aucune dépendance au CDN.
#
# Pré-requis : python3 (+ deps de peppol_api : dnspython, cryptography), node/npm,
# et un Chromium. Détection auto du binaire ; sinon exportez CHROME=/chemin/chrome
# (ou installez-en un : npx playwright install chromium).
#
# Usage :  bash docs/make_swagger_png.sh
#
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../server" && pwd)"
BUILD="$HERE/.swagger-build"          # cache npm + fichiers temporaires (gitignoré)
OUT="$HERE/swagger.png"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31mERREUR:\033[0m %s\n' "$*" >&2; exit 1; }

# --- 1. localiser Chromium ---------------------------------------------------
find_chrome() {
    [ -n "${CHROME:-}" ] && [ -x "${CHROME:-}" ] && { echo "$CHROME"; return; }
    local base c
    for base in "${PLAYWRIGHT_BROWSERS_PATH:-}" "$HOME/.cache/ms-playwright"; do
        [ -n "$base" ] || continue
        c=$(ls -d "$base"/chromium-*/chrome-linux/chrome 2>/dev/null | sort -V | tail -1 || true)
        [ -n "$c" ] && [ -x "$c" ] && { echo "$c"; return; }
        c=$(ls -d "$base"/chromium-*/chrome-mac*/Chromium.app/Contents/MacOS/Chromium 2>/dev/null | tail -1 || true)
        [ -n "$c" ] && { echo "$c"; return; }
    done
    for c in chromium chromium-browser google-chrome google-chrome-stable; do
        command -v "$c" >/dev/null 2>&1 && { command -v "$c"; return; }
    done
    echo ""
}
CHROME="$(find_chrome)"
[ -n "$CHROME" ] || die "Chromium introuvable. Exportez CHROME=/chemin/vers/chrome, ou : npx playwright install chromium"
log "Chromium : $CHROME"

command -v npm >/dev/null 2>&1 || die "npm requis (node/npm)."
command -v python3 >/dev/null 2>&1 || die "python3 requis."

# --- 2. dépendances npm (swagger-ui-dist + playwright-core) -------------------
log "Préparation des assets (npm : swagger-ui-dist, playwright-core)…"
mkdir -p "$BUILD"
( cd "$BUILD"
  [ -f package.json ] || npm init -y >/dev/null 2>&1
  # Erreurs npm laissées visibles (cache froid, réseau…) ; silencieux si OK.
  PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --no-audit --no-fund --loglevel=error \
      playwright-core swagger-ui-dist )

# --- 3. spec OpenAPI courante ------------------------------------------------
log "Génération de la spec OpenAPI (peppol_api.openapi_spec())…"
python3 -c "import sys, json; sys.path.insert(0, '$ROOT'); import peppol_api; \
print(json.dumps(peppol_api.openapi_spec(), ensure_ascii=False))" > "$BUILD/openapi.json" \
    || die "Import de peppol_api impossible (deps ? pip install dnspython cryptography)."

# --- 4. page Swagger autonome (spec inline : fetch file:// bloqué par Chromium)
{
  cat <<'H1'
<!doctype html><html lang="fr"><head><meta charset="utf-8"/>
<link rel="stylesheet" href="node_modules/swagger-ui-dist/swagger-ui.css"/>
<style>body{margin:0;background:#fafafa}.topbar{display:none}</style></head>
<body><div id="swagger-ui"></div>
<script src="node_modules/swagger-ui-dist/swagger-ui-bundle.js"></script>
<script src="node_modules/swagger-ui-dist/swagger-ui-standalone-preset.js"></script>
<script>const SPEC =
H1
  cat "$BUILD/openapi.json"
  cat <<'H2'
;
window.ui = SwaggerUIBundle({spec: SPEC, dom_id: '#swagger-ui', deepLinking: true,
  docExpansion: 'list', defaultModelsExpandDepth: 0, tryItOutEnabled: false,
  presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset]});
</script></body></html>
H2
} > "$BUILD/swagger.html"

# --- 5. rendu + capture ------------------------------------------------------
cat > "$BUILD/shoot.js" <<'JS'
const { chromium } = require('playwright-core');
(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME,
    args: ['--no-sandbox', '--force-color-profile=srgb'],
  });
  const page = await browser.newPage({ viewport: { width: 1100, height: 1400 }, deviceScaleFactor: 2 });
  await page.goto('file://' + __dirname + '/swagger.html', { waitUntil: 'networkidle' });
  await page.waitForSelector('.opblock', { timeout: 20000 });
  await page.waitForTimeout(600);
  await (await page.$('#swagger-ui')).screenshot({ path: process.env.OUT });
  await browser.close();
})().catch(e => { console.error('ERR', e.message); process.exit(1); });
JS

log "Rendu Swagger UI + capture…"
CHROME="$CHROME" OUT="$OUT" node "$BUILD/shoot.js"

log "OK → $OUT"
