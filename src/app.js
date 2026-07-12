const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open, save } = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);

/** Construit un élément DOM. Les enfants chaîne deviennent des nœuds texte :
 *  les données dynamiques (CSV, erreurs) ne passent JAMAIS par innerHTML. */
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
  try {
    const p = await invoke("preview_csv", { path });
    state.inputPath = path;
    state.preview = p;
    state.config.input = {
      path, delimiter: p.delimiter, encoding: p.encoding,
      pid_column: p.suggested_pid_column != null ? p.headers[p.suggested_pid_column] : "",
    };
    // Mapping par défaut : toutes les colonnes d'entrée + les 4 champs Peppol.
    state.config.output.columns = [
      ...p.headers.map((name) => ({ source: "input", name })),
      { source: "peppol", field: "exists" },
      { source: "peppol", field: "pa_code" },
      { source: "peppol", field: "pa_country" },
      { source: "peppol", field: "extended_ctc_fr" },
    ];
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

$("btn-browse").addEventListener("click", async () => {
  const f = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "txt"] }] });
  if (f) pickInput(f);
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
