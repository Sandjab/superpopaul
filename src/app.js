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
    api: { url: "https://peppol.gavini.cloud", key: "", batch_size: 50,
           concurrency: 8, proxy: null, refresh_days: 30 },
    input: { path: "", delimiter: ";", encoding: "utf-8", pid_column: "" },
    output: { path: "", timestamp_suffix: true, columns: [] },
  },
};

// --- Wizard --------------------------------------------------------------------
const STEPS = ["file", "columns", "output", "run"];
let current = 0;

function showStep(i) {
  // Cliquer l'onglet déjà actif ne doit rien faire (sinon ça re-affiche un
  // état périmé par-dessus des éditions non synchronisées).
  if (i === current) return;
  // En quittant l'étape output (Suivant, Précédent ou stepper), persister le
  // formulaire dans l'état pour ne pas perdre clé/URL modifiées.
  if (STEPS[current] === "output" && i !== current) syncOutputForm();
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
  if (STEPS[i] === "output") fillOutputForm(); // affiche le chemin suggéré par pickInput
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
  if (s === "output") {
    syncOutputForm();
    if (!state.config.output.path) return "Indique le fichier de sortie.";
    if (!state.config.api.key) return "Saisis la clé API (bouton Tester pour vérifier).";
  }
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
    // Mapping par défaut : toutes les colonnes d'entrée + les 4 champs Peppol.
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
        { source: "peppol", field: "pa_code" },
        { source: "peppol", field: "pa_country" },
        { source: "peppol", field: "extended_ctc_fr" },
      ];
    }
    if (!state.config.output.path)
      state.config.output.path = path.replace(/\.csv$/i, "") + "_enrichi.csv";
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
$("pid-column").addEventListener("change", (e) => { state.config.input.pid_column = e.target.value; });
const dz = $("dropzone");
dz.addEventListener("dragover", (e) => { e.preventDefault(); dz.classList.add("over"); });
dz.addEventListener("dragleave", () => dz.classList.remove("over"));
// Le drop de fichier natif arrive par l'événement Tauri drag-drop.
listen("tauri://drag-drop", (e) => {
  const paths = e.payload.paths || [];
  if (paths.length && STEPS[current] === "file") pickInput(paths[0]);
  dz.classList.remove("over");
});

// --- Étape 3 : formulaire ↔ état ---------------------------------------------------
function syncOutputForm() {
  const c = state.config;
  c.output.path = $("out-path").value.trim();
  c.output.timestamp_suffix = $("out-stamp").checked;
  c.api.url = $("api-url").value.trim();
  c.api.key = $("api-key").value.trim();
  const proxyUrl = $("proxy-url").value.trim();
  c.api.proxy = proxyUrl ? { url: proxyUrl } : null;
  c.api.concurrency = +$("api-conc").value || 8;
  c.api.batch_size = +$("api-batch").value || 50;
  c.api.refresh_days = +$("api-refresh").value || 30;
}
function fillOutputForm() {
  const c = state.config;
  $("out-path").value = c.output.path;
  $("out-stamp").checked = c.output.timestamp_suffix;
  $("api-url").value = c.api.url;
  $("api-key").value = c.api.key;
  $("proxy-url").value = c.api.proxy ? c.api.proxy.url : "";
  $("api-conc").value = c.api.concurrency;
  $("api-batch").value = c.api.batch_size;
  $("api-refresh").value = c.api.refresh_days;
}
$("btn-out-browse").addEventListener("click", async () => {
  const f = await save({ filters: [{ name: "CSV", extensions: ["csv"] }] });
  if (f) $("out-path").value = f;
});

// --- Splash --------------------------------------------------------------------------
window.addEventListener("DOMContentLoaded", () => {
  fillOutputForm();
  setTimeout(() => $("splash").classList.add("fade"), 700);
});

// --- Étape 3 : test API et calibrage -----------------------------------------
// Les deux flux partagent la config et la modale proxy : chaque flux désactive
// les DEUX boutons (exclusion mutuelle), pas seulement celui cliqué.
const apiButtons = () => [$("btn-test-api"), $("btn-calibrate")];

$("btn-test-api").addEventListener("click", async () => {
  apiButtons().forEach((b) => { b.disabled = true; });
  syncOutputForm();
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
  syncOutputForm();
  const out = $("calibrate-result");
  out.textContent = "calibrage en cours…";
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    const r = await invoke("calibrate_api");
    $("api-conc").value = r.best_concurrency;
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

// --- Config YAML : sauvegarde / chargement -------------------------------------
$("btn-save-cfg").addEventListener("click", async () => {
  // Le champ out-path n'est peuplé (fillOutputForm) qu'une fois l'étape
  // Sortie & API atteinte — gating du stepper. Avant ça, synchroniser
  // écraserait la config avec des champs de formulaire encore vides.
  if ($("out-path").value) syncOutputForm();
  // Pas de sauvegarde de config-squelette (décision produit) : sans fichier,
  // colonne d'adressage ou sortie, le YAML ne serait pas rechargeable.
  if (!state.inputPath || !state.config.input.pid_column || !state.config.output.path) {
    banner("warn", "Complète d'abord la configuration (fichier, colonne d'adressage, sortie) " +
      "avant de sauvegarder.");
    return;
  }
  const f = await save({ filters: [{ name: "YAML", extensions: ["yaml", "yml"] }] });
  if (!f) return;
  try {
    await invoke("save_config", { path: f, cfg: state.config });
    banner("warn", "⚠️ Config enregistrée — la clé API y est stockée en clair. " +
      "Ne partage ce fichier qu'avec des collègues de confiance.");
  } catch (e) {
    banner("error", `${e}`);
  }
});

$("btn-load-cfg").addEventListener("click", async () => {
  const f = await open({ multiple: false, filters: [{ name: "YAML", extensions: ["yaml", "yml"] }] });
  if (!f) return;
  try {
    state.config = await invoke("load_config", { path: f });
  } catch (e) {
    banner("error", `Chargement impossible : ${e}`);
    return;
  }
  fillOutputForm();
  let path = state.config.input.path;
  try {
    // Recharge l'aperçu du fichier d'entrée SANS écraser le mapping du YAML.
    path = await invoke("resolved_input_path");
    state.preview = await invoke("preview_csv", { path });
    state.inputPath = path;
    renderFilePanel();
    hideBanner();
    // Directement à l'étape Run (spec) — analyze_input y détecte la reprise.
    // showStep() a un early-return si on est déjà sur l'étape courante (cas du
    // clic sur l'onglet actif) : quand on charge un YAML depuis l'étape Run,
    // ce serait un no-op et enterRunStep() (donc analyze_input et la bannière
    // de reprise) ne serait jamais rappelé. On force donc l'entrée dans
    // l'étape Run dans ce cas précis, plutôt que de passer par showStep().
    const runIdx = STEPS.indexOf("run");
    if (current === runIdx) enterRunStep();
    else showStep(runIdx);
  } catch {
    // Config chargée mais CSV introuvable/illisible : la config reste en
    // place (l'utilisateur ne re-choisit que le fichier), l'état d'entrée
    // est remis à zéro pour rester cohérent et actionnable.
    state.inputPath = null;
    state.preview = null;
    banner("warn", `Config chargée, mais le fichier d'entrée ${path} est introuvable — ` +
      "re-sélectionne-le à l'étape 1.");
    showStep(0);
  }
});
