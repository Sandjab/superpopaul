const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open, save } = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);

/** Construit un élément DOM. Les enfants chaîne deviennent des nœuds texte :
 *  les données dynamiques (CSV, erreurs) ne passent JAMAIS par innerHTML.
 *  Attention : les valeurs d'attributs passent par setAttribute sans filtrage —
 *  ne jamais construire href/src depuis des données CSV/API. */
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else el.setAttribute(k, v);
  }
  el.append(...children);
  return el;
}

// --- État global -------------------------------------------------------------
const state = {
  inputPath: null,
  preview: null, // {headers, rows, delimiter, encoding, columns_hash, size_bytes, suggested_pid_column}
  config: {
    version: 1,
    // Résolveur direct par défaut : 8.8.8.8 avec 1.1.1.1 en secours (failover
    // du DNS classique) — le résolveur du FAI rate-limite sous rafale.
    api: { url: "https://peppol.gavini.cloud", key: "", mode: "api", resolver: "8.8.8.8",
           resolver_fallback: "1.1.1.1", dns_concurrency: 32,
           batch_size: 50, concurrency: 8, proxy: null, refresh_days: 30 },
    input: { path: "", delimiter: ";", encoding: "utf-8", pid_column: "" },
    output: { dir: "", suffix: "_enrichi", timestamp_suffix: true,
              encoding: "utf-8-bom", separator: "auto", columns: [] },
  },
};

// --- Wizard --------------------------------------------------------------------
const STEPS = ["file", "columns", "run"];
let current = 0;

function showStep(i) {
  // Cliquer l'onglet déjà actif ne doit rien faire (sinon ça re-affiche un
  // état périmé par-dessus des éditions non synchronisées).
  if (i === current) return;
  current = i;
  STEPS.forEach((s, j) => {
    $(`step-${s}`).classList.toggle("hidden", j !== i);
    const btn = document.querySelector(`#stepper [data-step="${s}"]`);
    btn.classList.toggle("active", j === i);
    btn.classList.toggle("done", j < i);
    if (j <= i) btn.disabled = false;
  });
  syncStepperGating(); // l'état a le dernier mot sur le déverrouillage par progression
  $("btn-prev").classList.toggle("hidden", i === 0);
  syncNextBtn();
  if (STEPS[i] === "columns") { renderPidSelect(); fillOutFormat(); renderOutPreview(); }
  if (STEPS[i] === "run") enterRunStep();          // cockpit.js
}

/** « Suivant » n'apparaît à l'étape Fichier qu'une fois un fichier chargé —
 *  avant, il n'y a pas d'étape suivante atteignable. */
function syncNextBtn() {
  const hide = current === STEPS.length - 1
    || (STEPS[current] === "file" && !state.inputPath);
  $("btn-next").classList.toggle("hidden", hide);
}
syncNextBtn(); // état initial : étape Fichier, aucun fichier

/** Le stepper suit l'état, pas seulement la progression : Format exige un
 *  fichier, Run exige une désignation — re-verrouillés si l'état régresse. */
function syncStepperGating() {
  document.querySelector('#stepper [data-step="columns"]').disabled = !state.inputPath;
  document.querySelector('#stepper [data-step="run"]').disabled =
    !state.inputPath || !state.config.input.pid_column;
}

/** Message d'erreur si l'étape courante est incomplète, sinon null. */
function validateStep() {
  const s = STEPS[current];
  if (s === "file" && !state.inputPath) return "Choisis d'abord un fichier CSV.";
  if (s === "columns" && !state.config.input.pid_column)
    return "Désigne la colonne des adressages (🔑).";
  // La clé API (mode api) est vérifiée au lancement du run (cockpit.js),
  // les réglages n'étant plus une étape du wizard.
  return null;
}

$("btn-next").addEventListener("click", () => {
  const err = validateStep();
  if (err) return banner("warn", err);
  hideBanner();
  showStep(current + 1);
});
$("btn-prev").addEventListener("click", () => { hideBanner(); showStep(current - 1); });
document.querySelectorAll("#stepper .step").forEach((b, j) =>
  b.addEventListener("click", () => !b.disabled && showStep(j)));

// --- Bannière / modale (textContent + nœuds : jamais d'innerHTML) --------------
function banner(kind, text, ...actionNodes) {
  const el = $("banner");
  el.className = kind;
  el.replaceChildren(text, ...actionNodes);
}
function hideBanner() { $("banner").className = "hidden"; }
function modal(...nodes) {
  $("modal").replaceChildren(...nodes);
  $("modal-backdrop").classList.remove("hidden");
}
function closeModal() { $("modal-backdrop").classList.add("hidden"); }

// --- Étape 1 : fichier -----------------------------------------------------------
async function pickInput(path) {
  // Garde léger : le dialogue filtre déjà csv/txt, mais le drag-drop accepte
  // n'importe quel chemin (un YAML déposé serait sniffé en séparateur « | »).
  if (!/\.(csv|txt)$/i.test(path)) {
    banner("warn", `Ce fichier n'est pas un CSV (.csv ou .txt attendu) : ${path}`);
    return;
  }
  try {
    const p = await invoke("preview_csv", { path });
    const prevHeaders = state.preview ? state.preview.headers : null;
    state.inputPath = path;
    state.preview = p;
    state.config.input = {
      path, delimiter: p.delimiter, encoding: p.encoding,
      pid_column: p.suggested_pid_column != null ? p.headers[p.suggested_pid_column] : "",
    };
    // Mapping par défaut : toutes les colonnes d'entrée + existe/CTC-FR ; les
    // autres champs Peppol démarrent dans la drop zone de l'étape 2.
    // Préserve un mapping personnalisé quand on re-choisit le même fichier :
    // on ne le reconstruit que si aucune colonne n'existe encore, ou si les
    // entêtes du nouveau fichier diffèrent de celles de l'ancien preview.
    const headersChanged = !prevHeaders
      || prevHeaders.length !== p.headers.length
      || prevHeaders.some((name, i) => name !== p.headers[i]);
    if (state.config.output.columns.length === 0 || headersChanged) {
      state.config.output.columns = [
        ...p.headers.map((name) => ({ source: "input", name })),
        { source: "peppol", field: "in_peppol" },
        { source: "peppol", field: "ubl_extended" },
      ];
    }
    // Invariant « adressage obligatoire en sortie » : la pré-désignation doit
    // réintégrer la colonne si un mapping conservé l'avait écartée.
    const pid = state.config.input.pid_column;
    if (pid && !state.config.output.columns.some((c) => c.source === "input" && c.name === pid))
      state.config.output.columns.push({ source: "input", name: pid });
    // output.dir vide = « dossier du fichier d'entrée » (résolu côté Rust) :
    // pas de valeur à poser ici, le réglage persiste d'un fichier à l'autre.
    renderFilePanel();
    hideBanner();
  } catch (e) {
    banner("error", `Impossible de lire ce fichier : ${e}`);
  }
}

function renderFilePanel() {
  const p = state.preview;
  syncNextBtn(); // un fichier vient d'être chargé : « Suivant » devient utile
  $("file-info").classList.remove("hidden");
  const meta = $("file-meta");
  meta.replaceChildren(
    h("b", {}, state.inputPath.split(/[\\/]/).pop() ?? ""),
    ` — ${Math.max(1, Math.round(p.size_bytes / 1024))} Ko · séparateur « ${p.delimiter} », encodage ${p.encoding}`);
  meta.title = state.inputPath;
  $("preview-table").replaceChildren(
    h("tr", {}, ...p.headers.map((hd) => h("th", {}, hd))),
    ...p.rows.slice(0, 3).map((r) => h("tr", {}, ...r.map((c) => h("td", {}, c)))),
  );
  highlightPidColumn();
  syncStepperGating();
}

/** Liste de désignation de l'étape Format — miroir de state…pid_column.
 *  Sans désignation (aucune suggestion) : placeholder « — choisir — ». */
function renderPidSelect() {
  const headers = state.preview ? state.preview.headers : [];
  const opts = headers.map((hd) => {
    const o = h("option", { value: hd }, hd);
    o.selected = hd === state.config.input.pid_column;
    return o;
  });
  if (!state.config.input.pid_column) {
    const ph = h("option", { value: "" }, "— choisir —");
    ph.selected = true;
    ph.disabled = true;
    opts.unshift(ph);
  }
  $("pid-column").replaceChildren(...opts);
  $("pid-hint").textContent =
    state.preview && state.preview.suggested_pid_column != null
      ? "(suggestion automatique)" : "";
  // Un profil sans désignation serait invalide : sauvegarde grisée.
  $("btn-save-cfg").disabled = !state.config.input.pid_column;
}

/** Désignation — LE point d'entrée unique (liste ou clé 🔑 du tableau).
 *  La colonne désignée est obligatoire en sortie : si elle était écartée,
 *  elle est réintégrée d'office ; l'ancienne redevient écartable. */
function designatePid(name) {
  state.config.input.pid_column = name;
  const cols = state.config.output.columns;
  if (!cols.some((c) => c.source === "input" && c.name === name))
    cols.push({ source: "input", name });
  renderPidSelect();
  renderOutPreview();
  highlightPidColumn();
  syncStepperGating();
}

/** Surligne dans l'aperçu la colonne des adressages choisie (couleur d'accent,
 *  même langage visuel que les colonnes Peppol de l'étape 2). */
function highlightPidColumn() {
  const idx = state.preview
    ? state.preview.headers.indexOf(state.config.input.pid_column) : -1;
  document.querySelectorAll("#preview-table tr").forEach((tr) =>
    [...tr.children].forEach((cell, i) => cell.classList.toggle("pid-col", i === idx)));
}

$("btn-browse").addEventListener("click", async (e) => {
  const btn = e.currentTarget;
  btn.disabled = true; // garde de ré-entrance pendant le dialog + preview
  try {
    const f = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "txt"] }] });
    if (f) await pickInput(f);
  } finally {
    btn.disabled = false;
  }
});
$("pid-column").addEventListener("change", (e) => designatePid(e.target.value));
const dz = $("dropzone");
dz.addEventListener("dragover", (e) => { e.preventDefault(); dz.classList.add("over"); });
dz.addEventListener("dragleave", () => dz.classList.remove("over"));
// Le drop de fichier natif arrive par l'événement Tauri drag-drop.
listen("tauri://drag-drop", (e) => {
  const paths = e.payload.paths || [];
  if (paths.length && STEPS[current] === "file") pickInput(paths[0]);
  dz.classList.remove("over");
});

// --- Réglages : formulaire ↔ état ---------------------------------------------------
function syncSettingsForm() {
  const c = state.config;
  c.output.dir = $("out-dir").value.trim();
  c.output.suffix = $("out-suffix").value.trim();
  c.output.timestamp_suffix = $("out-stamp").checked;
  c.api.mode = $("api-mode").value;
  c.api.url = $("api-url").value.trim();
  c.api.key = $("api-key").value.trim();
  // Case DoH = aide de saisie : une IP cochée DoH est enregistrée sous sa
  // forme canonique https://<ip>/dns-query — résolveur ET secours, qui doit
  // être de même nature que le principal (l'interprétation des champs —
  // vide/IP/URL, panachage refusé — reste côté Rust, parse_resolver_spec).
  let resolver = $("dns-resolver").value.trim();
  let fallback = $("dns-fallback").value.trim();
  if (resolver && $("dns-doh").checked) {
    if (!resolver.startsWith("https://")) resolver = `https://${resolver}/dns-query`;
    if (fallback && !fallback.startsWith("https://")) fallback = `https://${fallback}/dns-query`;
  }
  c.api.resolver = resolver || null;
  c.api.resolver_fallback = fallback;
  c.api.dns_concurrency = +$("dns-conc").value || 32;
  // Deux champs Concurrence (un par bloc de mode), miroirs l'un de l'autre :
  // on lit celui du mode courant.
  c.api.concurrency =
    +(c.api.mode === "direct" ? $("direct-conc") : $("api-conc")).value || 8;
  c.api.batch_size = +$("api-batch").value || 50;
  const proxyUrl = $("proxy-url").value.trim();
  c.api.proxy = $("proxy-on").checked && proxyUrl ? { url: proxyUrl } : null;
  c.api.refresh_days = +$("api-refresh").value || 30;
}
function fillSettingsForm() {
  const c = state.config;
  $("out-dir").value = c.output.dir;
  $("out-suffix").value = c.output.suffix;
  $("out-stamp").checked = c.output.timestamp_suffix;
  $("api-mode").value = c.api.mode || "api";
  $("api-url").value = c.api.url;
  $("api-key").value = c.api.key;
  $("dns-resolver").value = c.api.resolver || "";
  $("dns-doh").checked = (c.api.resolver || "").startsWith("https://");
  $("dns-fallback").value = c.api.resolver_fallback ?? "1.1.1.1";
  $("dns-conc").value = c.api.dns_concurrency || 32;
  $("api-conc").value = c.api.concurrency;
  $("direct-conc").value = c.api.concurrency;
  $("api-batch").value = c.api.batch_size;
  $("proxy-on").checked = !!c.api.proxy;
  $("proxy-url").value = c.api.proxy ? c.api.proxy.url : "";
  $("api-refresh").value = c.api.refresh_days;
  syncModeUi();
  syncProxyUi();
  syncDnsUi();
}

/** Affiche le bloc de champs du backend choisi (API ou direct), et l'aide
 *  visible correspondante (l'info de décision ne vit pas en tooltip). */
const API_MODE_HINTS = {
  api: "Résolution en lots via le serveur Popaul — clé d'API requise.",
  direct: "SML + SMP interrogés depuis ce poste, sans clé ni serveur — un adressage à la fois.",
};
function syncModeUi() {
  const direct = $("api-mode").value === "direct";
  $("api-fields").classList.toggle("hidden", direct);
  $("direct-fields").classList.toggle("hidden", !direct);
  $("api-mode-hint").textContent = API_MODE_HINTS[$("api-mode").value] ?? "";
  if (direct) $("api-test-result").textContent = "";
}
$("api-mode").addEventListener("change", syncModeUi);

/** Le secours ne sert qu'au DNS classique : grisé en DNS système (champ
 *  vide) comme en DoH (case cochée ou URL saisie) — la valeur reste
 *  enregistrée, Rust l'ignore hors mode classique. */
/** Le secours suit le principal (IP ou DoH) : grisé seulement sans résolveur
 *  choisi (DNS système, où il n'a pas de sens). */
function syncDnsUi() {
  $("dns-fallback").disabled = !$("dns-resolver").value.trim();
}
$("dns-resolver").addEventListener("input", syncDnsUi);

/** Grise toute la zone Proxy tant que la case (dans la légende, donc épargnée
 *  par le disabled natif du fieldset) n'est pas cochée. */
function syncProxyUi() {
  $("proxy-zone").disabled = !$("proxy-on").checked;
}
$("proxy-on").addEventListener("change", syncProxyUi);

// Les deux champs Concurrence pilotent la même valeur : les garder miroirs
// pour qu'un changement de mode ne fasse pas resurgir une ancienne saisie.
$("api-conc").addEventListener("input", () => { $("direct-conc").value = $("api-conc").value; });
$("direct-conc").addEventListener("input", () => { $("api-conc").value = $("direct-conc").value; });

$("btn-out-browse").addEventListener("click", async () => {
  const d = await open({ directory: true });
  if (d) $("out-dir").value = d;
});

// --- Étape Format : forme de sortie (encodage, séparateur) --------------------
function fillOutFormat() {
  $("out-encoding").value = state.config.output.encoding;
  $("out-sep").value = state.config.output.separator;
}
$("out-encoding").addEventListener("change", (e) => { state.config.output.encoding = e.target.value; });
$("out-sep").addEventListener("change", (e) => { state.config.output.separator = e.target.value; });

// --- Réglages : persistance (superpopaul.yaml, dossier données de l'app) -----------
/** La tranche de l'état qui va dans le fichier de réglages : API + forme de la
 *  sortie. Ni le fichier d'entrée ni les colonnes (ça, c'est le profil). */
function currentSettings() {
  const c = state.config;
  const { dir, suffix, timestamp_suffix } = c.output;
  return { version: 1, api: c.api,
           output: { dir, suffix, timestamp_suffix } };
}
/** Fusion sur les défauts de l'état : les champs à leur valeur par défaut sont
 *  absents du YAML (serde skip_serializing_if), un remplacement les perdrait. */
function applySettings(s) {
  Object.assign(state.config.api, s.api);
  Object.assign(state.config.output, s.output);
}

// --- Réglages : ouverture / fermeture ------------------------------------------------
function openSettings() {
  fillSettingsForm();
  $("settings-error").classList.add("hidden");
  $("settings-backdrop").classList.remove("hidden");
}
async function closeSettings() {
  syncSettingsForm();
  // Auto-enregistrement à la fermeture. En cas de refus (suffixe invalide…),
  // le panneau reste ouvert avec l'erreur — la bannière du haut serait
  // recouverte par l'overlay.
  try {
    await invoke("save_settings", { settings: currentSettings() });
  } catch (e) {
    const err = $("settings-error");
    err.textContent = `Réglages non enregistrés : ${e}`;
    err.classList.remove("hidden");
    return;
  }
  $("settings-backdrop").classList.add("hidden");
  // L'ancienneté refresh a pu changer : l'aide du mode de run la cite.
  window.updateRunModeHint?.();
}
$("btn-settings").addEventListener("click", openSettings);
$("btn-settings-close").addEventListener("click", closeSettings);
$("settings-backdrop").addEventListener("click", (e) => {
  if (e.target === $("settings-backdrop")) closeSettings();
});
document.addEventListener("keydown", (e) => {
  // Échap ferme les réglages — sauf si la modale (proxy) est ouverte au-dessus,
  // auquel cas c'est son propre handler qui gère la touche.
  if (e.key === "Escape"
      && !$("settings-backdrop").classList.contains("hidden")
      && $("modal-backdrop").classList.contains("hidden")) closeSettings();
});

// --- Splash + réglages au démarrage ---------------------------------------------------
window.addEventListener("DOMContentLoaded", async () => {
  setTimeout(() => $("splash").classList.add("fade"), 2000);
  // Version du programme dans le pied de page (celle de tauri.conf.json).
  window.__TAURI__.app?.getVersion().then((v) => { $("app-version").textContent = `v${v}`; });
  // Réglages auto-persistés : lus au démarrage (premier lancement : défauts).
  try {
    const s = await invoke("load_settings");
    if (s) applySettings(s);
  } catch (e) {
    banner("warn", `Réglages illisibles — valeurs par défaut appliquées. (${e})`);
  }
  fillSettingsForm();
});

// Lien externe : toujours via le navigateur par défaut du système (opener),
// jamais dans la webview — un <a href> nu y naviguerait l'app elle-même.
$("brand-link").addEventListener("click", (e) => {
  e.preventDefault();
  window.__TAURI__.opener?.openUrl("https://github.com/Sandjab/superpopaul");
});

// --- Réglages : test API et calibrage -----------------------------------------
// Les deux flux partagent la config et la modale proxy : chaque flux désactive
// les DEUX boutons (exclusion mutuelle), pas seulement celui cliqué.
const apiButtons = () => [$("btn-test-api"), $("btn-calibrate")];

$("btn-test-api").addEventListener("click", async () => {
  apiButtons().forEach((b) => { b.disabled = true; });
  syncSettingsForm();
  const out = $("api-test-result");
  out.textContent = "test en cours…";
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    const stats = await invoke("test_api");
    out.textContent = `✅ clé valide (${stats.latency_ms} ms)`;
  } catch (err) {
    if (err && err.proxyCancelled) out.textContent = "Test annulé.";
    else {
      // Échec d'auth proxy probable : re-demander les identifiants au prochain clic.
      if (/407|proxy/i.test(String(err))) proxyCredsGiven = false;
      out.textContent = `❌ ${err}`;
    }
  } finally {
    apiButtons().forEach((b) => { b.disabled = false; });
  }
});

// --- Banc d'essai du calibrage : une colonne par palier, hauteurs re-échelonnées
// sur le meilleur débit vu (le backend n'envoie que des adr/s absolus).
const bench = { el: null, statusEl: null, cols: new Map(), max: 0, steps: [] };

function benchReset(el) {
  bench.el = el;
  el.replaceChildren();
  bench.cols.clear();
  bench.max = 0;
  bench.steps = [];
}

function benchRescale() {
  for (const { bar } of bench.cols.values()) {
    const v = Number(bar.dataset.adr || 0);
    if (v > 0 && bench.max > 0)
      bar.style.height = `${Math.max(4, Math.round((v / bench.max) * 52))}px`;
  }
}

listen("calibrate-step", (e) => {
  if (!bench.el) return; // événement orphelin (modale déjà fermée)
  const s = e.payload;
  bench.steps.push(s);
  if (s.status === "measuring") {
    if (bench.statusEl)
      bench.statusEl.textContent = `palier ${s.level} session${s.level > 1 ? "s" : ""} — mesure…`;
    const val = h("span", { class: "cal-val" }, "");
    const bar = h("div", { class: "cal-bar measuring" });
    const col = h("div", { class: "cal-col" }, val, bar,
      h("span", { class: "cal-lab" }, String(s.level)));
    bench.cols.set(s.level, { col, bar, val });
    bench.el.append(col);
    return;
  }
  const entry = bench.cols.get(s.level);
  if (!entry) return;
  entry.bar.classList.remove("measuring");
  entry.bar.dataset.adr = String(s.addr_per_s);
  entry.val.textContent = String(Math.round(s.addr_per_s));
  if (s.addr_per_s > bench.max) bench.max = s.addr_per_s;
  if (s.status === "retained") {
    // Le vert bascule : l'ancien meilleur redevient gris.
    for (const { col, bar } of bench.cols.values()) {
      col.classList.remove("win");
      bar.classList.remove("win");
    }
    entry.col.classList.add("win");
    entry.bar.classList.add("win");
  } else if (s.status === "rejected") {
    entry.col.classList.add("reject");
    entry.bar.classList.add("reject");
  } else if (s.status === "rate_limited") {
    entry.col.classList.add("ratelimited");
    entry.bar.classList.add("ratelimited");
  }
  benchRescale();
});

/** Raison d'arrêt pour le verdict texte — formatage de présentation uniquement. */
function benchStopReason(r) {
  const last = bench.steps[bench.steps.length - 1];
  if (!last || last.status === "measuring") return r.rate_limited ? " (clé rate-limitée)" : "";
  if (last.status === "rate_limited") return ` (${last.level} : rate-limité, arrêt)`;
  if (last.status === "rejected") {
    const gain = r.addr_per_s > 0
      ? Math.floor((last.addr_per_s / r.addr_per_s - 1) * 100) : 0;
    return ` (${last.level} : ${gain >= 0 ? "+" : ""}${gain} % < 15 %, arrêt)`;
  }
  return ""; // arrêt par plafond : rien à expliquer
}

function benchDimLosers() {
  for (const { bar } of bench.cols.values()) {
    if (!bar.classList.contains("win") && !bar.classList.contains("reject")
        && !bar.classList.contains("ratelimited")
        && !bar.classList.contains("measuring")) bar.classList.add("dim");
  }
}

/** Flux complet de calibration dans la modale partagée. L'application de la
 *  concurrence est EXPLICITE (bouton Appliquer) — plus d'écriture automatique. */
async function runCalibration() {
  apiButtons().forEach((b) => { b.disabled = true; });
  syncSettingsForm();
  const out = $("calibrate-result");
  out.textContent = "calibration en cours…";
  const backdrop = $("modal-backdrop");
  let onBackdrop = null;
  let onKeydown = null;
  const cleanup = () => {
    if (onBackdrop) backdrop.removeEventListener("click", onBackdrop);
    if (onKeydown) document.removeEventListener("keydown", onKeydown);
    closeModal();
    bench.el = null;
    bench.statusEl = null;
    apiButtons().forEach((b) => { b.disabled = false; });
  };
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    // La modale ne s'ouvre qu'une fois les prérequis franchis côté UI ; une
    // erreur de garde backend (invoke rejeté) la referme dans le catch.
    const title = h("h3", {}, "Calibration en cours…");
    const benchEl = h("div", { id: "calibrate-bench" });
    // Légende visible sous le banc (pas en tooltip : c'est elle qui explique
    // les couleurs pendant la mesure).
    const legend = h("p", { class: "cal-legend" }, "adr/s par nombre de sessions —",
      h("span", { class: "dot", style: "background:var(--green)" }), "retenu",
      h("span", { class: "dot", style: "background:var(--red)" }), "gain < 15 %",
      h("span", { class: "dot", style: "background:var(--amber)" }), "rate-limité");
    const status = h("div", { id: "calibrate-status" }, "démarrage…");
    const btns = h("div", { class: "modal-btns" });
    modal(title, benchEl, legend, status, btns);
    benchReset(benchEl);
    bench.statusEl = status;
    const stopBtn = h("button", {
      class: "btn-danger",
      onclick: () => {
        stopBtn.disabled = true;
        stopBtn.textContent = "arrêt en cours…";
        invoke("cancel_calibration").catch(() => {}); // fire-and-forget assumé
      },
    }, "■ Arrêter");
    btns.append(stopBtn);

    const r = await invoke("calibrate_api");
    benchDimLosers();
    const verdict = (r.cancelled ? "arrêtée · " : "") +
      `→ ${r.best_concurrency} sessions, ~${Math.round(r.addr_per_s)} adr/s` +
      ` · ${r.addr_sent} adressages consommés` + benchStopReason(r);
    // Le rapport fait autorité (le dernier calibrate-step peut perdre la
    // course contre la résolution de l'invoke) : un best par défaut (1, 0.0)
    // — annulation immédiate ou palier 1 à zéro réussite — ne doit pas être
    // applicable.
    const hasComplete = r.addr_per_s > 0;
    if (r.cancelled) {
      title.textContent = "Calibration arrêtée";
      const last = bench.steps[bench.steps.length - 1];
      status.textContent = (last ? `arrêtée au palier ${last.level} · ` : "") +
        `meilleur mesuré : ${r.best_concurrency} sessions, ~${Math.round(r.addr_per_s)} adr/s` +
        ` · ${r.addr_sent} adressages consommés`;
    } else {
      title.textContent = "Calibration terminée";
      status.textContent = verdict;
      status.classList.add("done");
    }
    const finish = (applied) => {
      out.textContent = verdict + (applied ? " — appliquée" : "");
      cleanup();
    };
    const ignore = () => finish(false);
    onBackdrop = (e) => { if (e.target === backdrop) ignore(); };
    onKeydown = (e) => { if (e.key === "Escape") ignore(); };
    backdrop.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKeydown);
    const applyBtn = h("button", {
      class: "btn-primary",
      onclick: () => {
        $("api-conc").value = r.best_concurrency;
        $("direct-conc").value = r.best_concurrency; // champs miroirs
        state.config.api.concurrency = r.best_concurrency;
        finish(true);
      },
    }, `✓ Appliquer ${r.best_concurrency} sessions`);
    applyBtn.disabled = !hasComplete;
    btns.replaceChildren(
      h("button", { class: "btn-retry", onclick: () => { cleanup(); runCalibration(); } }, "↻ Retenter"),
      h("button", { onclick: ignore }, "Ignorer"),
      applyBtn,
    );
  } catch (err) {
    cleanup();
    if (err && err.proxyCancelled) out.textContent = "Calibration annulée.";
    else {
      // Échec d'auth proxy probable : re-demander les identifiants au prochain clic.
      if (/407|proxy/i.test(String(err))) proxyCredsGiven = false;
      out.textContent = `❌ ${err}`;
    }
  }
}
$("btn-calibrate").addEventListener("click", runCalibration);

/** Si un proxy est configuré et les identifiants pas encore saisis dans cette
 *  session — ou saisis pour une autre URL de proxy —, les demander (mémoire
 *  seulement — jamais persistés). Single-flight : si la modale est déjà
 *  ouverte, retourne la Promise en cours. L'annulation (bouton, Échap, clic
 *  sur le fond) rejette avec une erreur marquée `proxyCancelled`. */
let proxyCredsGiven = false;
let proxyCredsUrl = null; // URL proxy pour laquelle les identifiants ont été saisis
let pendingCreds = null; // Promise de la modale en cours (single-flight)
function ensureProxyCreds(force = false) {
  const proxy = state.config.api.proxy;
  if (!proxy) return Promise.resolve();
  if (proxyCredsGiven && proxyCredsUrl === proxy.url && !force) return Promise.resolve();
  if (pendingCreds) return pendingCreds;
  pendingCreds = new Promise((resolve, reject) => {
    const user = h("input", { placeholder: "login" });
    const pass = h("input", { type: "password", placeholder: "mot de passe" });
    const msg = h("p", { class: "muted" });
    const backdrop = $("modal-backdrop");
    // Tout chemin de sortie retire les listeners globaux (la modale est
    // partagée avec d'autres usages) et libère le single-flight avant de
    // régler la Promise.
    const settle = (fn, value) => {
      backdrop.removeEventListener("click", onBackdrop);
      document.removeEventListener("keydown", onKeydown);
      closeModal();
      pendingCreds = null;
      fn(value);
    };
    const cancel = () => {
      const err = new Error("Saisie des identifiants proxy annulée.");
      err.proxyCancelled = true;
      settle(reject, err);
    };
    const onBackdrop = (e) => { if (e.target === backdrop) cancel(); };
    const onKeydown = (e) => { if (e.key === "Escape") cancel(); };
    backdrop.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKeydown);
    modal(
      h("h3", {}, "Identifiants proxy"),
      h("p", { class: "muted" }, "Conservés en mémoire uniquement, jamais enregistrés."),
      user, pass, msg,
      h("button", {
        class: "btn-primary",
        onclick: async () => {
          if (!user.value.trim()) { msg.textContent = "Le login est obligatoire."; return; }
          await invoke("set_proxy_creds", { username: user.value, password: pass.value });
          proxyCredsGiven = true;
          proxyCredsUrl = proxy.url;
          settle(resolve);
        },
      }, "Valider"),
      h("button", { onclick: cancel }, "Annuler"),
    );
  });
  return pendingCreds;
}

// --- Profils de chargement : sauvegarde / chargement explicites -------------------
// Un profil décrit COMMENT traiter un fichier (colonne des adressages, signature
// de colonnes, colonnes de sortie, encodage/séparateur) ; sans chemin — un profil
// s'applique au fichier ouvert, pas à un chemin figé. Les réglages (API), eux,
// sont auto-persistés séparément.

// En mode portable les dialogues de profils s'ouvrent à côté de l'exe ;
// en mode installé, pas de defaultPath (dernier dossier visité, comportement OS).
async function profileDialogDefault() {
  const dir = await invoke("portable_dir").catch(() => null);
  return dir ? { defaultPath: dir } : {};
}

$("btn-save-cfg").addEventListener("click", async () => {
  const f = await save({ filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
                         ...(await profileDialogDefault()) });
  if (!f) return;
  try {
    await invoke("save_profile", { path: f, profile: {
      version: 1,
      input: { pid_column: state.config.input.pid_column,
               columns_hash: state.preview.columns_hash },
      output: { encoding: state.config.output.encoding,
                separator: state.config.output.separator },
      columns: state.config.output.columns,
    } });
    hideBanner();
  } catch (e) {
    banner("error", `${e}`);
  }
});

$("btn-load-cfg").addEventListener("click", async () => {
  const f = await open({ multiple: false, filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
                         ...(await profileDialogDefault()) });
  if (!f) return;
  let p;
  try {
    p = await invoke("load_profile", { path: f });
  } catch (e) {
    banner("error", `Chargement impossible : ${e}`);
    return;
  }
  // Refus sec : un profil forcé sur d'autres colonnes produirait une sortie
  // silencieusement fausse. Aucun état modifié.
  if (p.input.columns_hash !== state.preview.columns_hash) {
    banner("error", "Profil incompatible avec le fichier ouvert — colonnes différentes.");
    return;
  }
  state.config.input.pid_column = p.input.pid_column;
  state.config.output.columns = p.columns;
  state.config.output.encoding = p.output.encoding;
  state.config.output.separator = p.output.separator;
  hideBanner();
  renderPidSelect();
  fillOutFormat();
  renderOutPreview();
  highlightPidColumn();
  syncStepperGating();
});
