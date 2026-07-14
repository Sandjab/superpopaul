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
  preview: null, // {headers, rows, delimiter, encoding, suggested_pid_column}
  config: {
    version: 1,
    api: { url: "https://peppol.gavini.cloud", key: "", mode: "api", resolver: null, dns_concurrency: 32,
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
  $("btn-prev").classList.toggle("hidden", i === 0);
  $("btn-next").classList.toggle("hidden", i === STEPS.length - 1);
  if (STEPS[i] === "columns") renderOutPreview(); // columns.js
  if (STEPS[i] === "run") enterRunStep();          // cockpit.js
}

/** Message d'erreur si l'étape courante est incomplète, sinon null. */
function validateStep() {
  const s = STEPS[current];
  if (s === "file") {
    if (!state.inputPath) return "Choisis d'abord un fichier CSV.";
    if (!state.config.input.pid_column) return "Indique la colonne des adressages.";
  }
  if (s === "columns" && state.config.output.columns.length === 0)
    return "Il faut au moins une colonne en sortie.";
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
        { source: "peppol", field: "exists" },
        { source: "peppol", field: "extended_ctc_fr" },
      ];
    }
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
  $("file-info").classList.remove("hidden");
  $("file-meta").textContent =
    `${state.inputPath} — séparateur « ${p.delimiter} », encodage ${p.encoding}`;
  $("preview-table").replaceChildren(
    h("tr", {}, ...p.headers.map((hd) => h("th", {}, hd))),
    ...p.rows.map((r) => h("tr", {}, ...r.map((c) => h("td", {}, c)))),
  );
  $("pid-column").replaceChildren(...p.headers.map((hd) => {
    const o = h("option", {}, hd);
    o.selected = hd === state.config.input.pid_column;
    return o;
  }));
  $("pid-hint").textContent =
    p.suggested_pid_column != null ? "(suggestion automatique)" : "";
  highlightPidColumn();
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
$("pid-column").addEventListener("change", (e) => {
  state.config.input.pid_column = e.target.value;
  highlightPidColumn();
});
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
  c.output.encoding = $("out-encoding").value;
  c.output.separator = $("out-sep").value;
  c.api.mode = $("api-mode").value;
  c.api.url = $("api-url").value.trim();
  c.api.key = $("api-key").value.trim();
  // Case DoH = aide de saisie : une IP cochée DoH est enregistrée sous sa
  // forme canonique https://<ip>/dns-query (l'interprétation du champ —
  // vide/IP/URL — reste côté Rust, dns_from_spec).
  let resolver = $("dns-resolver").value.trim();
  if (resolver && $("dns-doh").checked && !resolver.startsWith("https://"))
    resolver = `https://${resolver}/dns-query`;
  c.api.resolver = resolver || null;
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
  $("out-encoding").value = c.output.encoding;
  $("out-sep").value = c.output.separator;
  $("api-mode").value = c.api.mode || "api";
  $("api-url").value = c.api.url;
  $("api-key").value = c.api.key;
  $("dns-resolver").value = c.api.resolver || "";
  $("dns-doh").checked = (c.api.resolver || "").startsWith("https://");
  $("dns-conc").value = c.api.dns_concurrency || 32;
  $("api-conc").value = c.api.concurrency;
  $("direct-conc").value = c.api.concurrency;
  $("api-batch").value = c.api.batch_size;
  $("proxy-on").checked = !!c.api.proxy;
  $("proxy-url").value = c.api.proxy ? c.api.proxy.url : "";
  $("api-refresh").value = c.api.refresh_days;
  syncModeUi();
  syncProxyUi();
}

/** Affiche le bloc de champs du backend choisi (API ou direct). */
function syncModeUi() {
  const direct = $("api-mode").value === "direct";
  $("api-fields").classList.toggle("hidden", direct);
  $("direct-fields").classList.toggle("hidden", !direct);
  if (direct) $("api-test-result").textContent = "";
}
$("api-mode").addEventListener("change", syncModeUi);

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

// --- Réglages : persistance (superpopaul.yaml, dossier données de l'app) -----------
/** La tranche de l'état qui va dans le fichier de réglages : API + forme de la
 *  sortie. Ni le fichier d'entrée ni les colonnes (ça, c'est le profil). */
function currentSettings() {
  const c = state.config;
  const { dir, suffix, timestamp_suffix, encoding, separator } = c.output;
  return { version: 1, api: c.api,
           output: { dir, suffix, timestamp_suffix, encoding, separator } };
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
  // Réglages auto-persistés : lus au démarrage (premier lancement : défauts).
  try {
    const s = await invoke("load_settings");
    if (s) applySettings(s);
  } catch (e) {
    banner("warn", `Réglages illisibles — valeurs par défaut appliquées. (${e})`);
  }
  fillSettingsForm();
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

$("btn-calibrate").addEventListener("click", async () => {
  apiButtons().forEach((b) => { b.disabled = true; });
  syncSettingsForm();
  const out = $("calibrate-result");
  out.textContent = "calibrage en cours…";
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    const r = await invoke("calibrate_api");
    $("api-conc").value = r.best_concurrency;
    $("direct-conc").value = r.best_concurrency; // champs miroirs
    state.config.api.concurrency = r.best_concurrency;
    out.textContent = `→ ${r.best_concurrency} sessions, ~${Math.round(r.addr_per_s)} adr/s` +
      (r.rate_limited ? " (clé rate-limitée)" : "");
  } catch (err) {
    if (err && err.proxyCancelled) out.textContent = "Calibrage annulé.";
    else {
      // Échec d'auth proxy probable : re-demander les identifiants au prochain clic.
      if (/407|proxy/i.test(String(err))) proxyCredsGiven = false;
      out.textContent = `❌ ${err}`;
    }
  } finally {
    apiButtons().forEach((b) => { b.disabled = false; });
  }
});

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
// Un profil décrit COMMENT traiter un fichier (chemin, colonne des adressages,
// colonnes de sortie) ; les réglages (API, sortie), eux, sont auto-persistés.
$("btn-save-cfg").addEventListener("click", async () => {
  if (!state.inputPath || !state.config.input.pid_column) {
    banner("warn", "Choisis d'abord le fichier et la colonne des adressages " +
      "avant de sauvegarder un profil.");
    return;
  }
  const f = await save({ filters: [{ name: "YAML", extensions: ["yaml", "yml"] }] });
  if (!f) return;
  try {
    await invoke("save_profile", { path: f, profile: {
      version: 1,
      input: { path: state.inputPath, pid_column: state.config.input.pid_column },
      columns: state.config.output.columns,
    } });
    hideBanner(); // plus de clé API dans le fichier : plus d'avertissement
  } catch (e) {
    banner("error", `${e}`);
  }
});

$("btn-load-cfg").addEventListener("click", async () => {
  const f = await open({ multiple: false, filters: [{ name: "YAML", extensions: ["yaml", "yml"] }] });
  if (!f) return;
  let r;
  try {
    r = await invoke("load_profile", { path: f });
  } catch (e) {
    banner("error", `Chargement impossible : ${e}`);
    return;
  }
  state.config.input.path = r.profile.input.path;
  state.config.input.pid_column = r.profile.input.pid_column;
  state.config.output.columns = r.profile.columns;
  let path = r.profile.input.path;
  try {
    // Pose la config assemblée AVANT resolved_input_path (qui lit la config
    // active), puis recharge l'aperçu SANS écraser le mapping du profil.
    await invoke("set_config", { cfg: state.config });
    path = await invoke("resolved_input_path");
    state.preview = await invoke("preview_csv", { path });
    state.inputPath = path;
    renderFilePanel();
    if (r.legacy)
      banner("warn", "Ancien format : seuls le fichier, la colonne des adressages et " +
        "les colonnes ont été repris — l'API et la sortie se règlent désormais dans ⚙.");
    else hideBanner();
    // Directement à l'étape Run (spec) — analyze_input y détecte la reprise.
    // showStep() a un early-return si on est déjà sur l'étape courante (cas du
    // clic sur l'onglet actif) : quand on charge un profil depuis l'étape Run,
    // ce serait un no-op et enterRunStep() (donc analyze_input et la bannière
    // de reprise) ne serait jamais rappelé. On force donc l'entrée dans
    // l'étape Run dans ce cas précis, plutôt que de passer par showStep().
    const runIdx = STEPS.indexOf("run");
    if (current === runIdx) enterRunStep();
    else showStep(runIdx);
  } catch {
    // Profil chargé mais CSV introuvable/illisible : le profil reste en
    // place (l'utilisateur ne re-choisit que le fichier), l'état d'entrée
    // est remis à zéro pour rester cohérent et actionnable.
    state.inputPath = null;
    state.preview = null;
    banner("warn", `Profil chargé, mais le fichier d'entrée ${path} est introuvable — ` +
      "re-sélectionne-le à l'étape 1.");
    showStep(0);
  }
});
