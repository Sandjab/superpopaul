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
  // Profil courant (session seulement) : chemin/nom du YAML et instantané de
  // référence (profileSnapshot) — null tant qu'aucun profil chargé/enregistré.
  profile: null, // { path, name, ref }
  config: {
    version: 1,
    // Résolveur direct par défaut : 8.8.8.8 avec 1.1.1.1 en secours (failover
    // du DNS classique) — le résolveur du FAI rate-limite sous rafale.
    api: { url: "https://peppol.gavini.org", key: "", mode: "api", resolver: "8.8.8.8",
           resolver_fallback: "1.1.1.1", dns_concurrency: 32,
           batch_size: 50, concurrency: 8, proxy: null, refresh_days: 30 },
    input: { path: "", delimiter: ";", encoding: "utf-8", pid_column: "", record_label: "cf" },
    output: { dir: "", suffix: "_enrichi", timestamp_suffix: true,
              encoding: "utf-8-bom", separator: ";", columns: [] },
    ppf: { active_motifs: "CP" },
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
    const prevPid = state.config.input.pid_column;
    // Le libellé « type d'enregistrement » est une préférence indépendante du
    // fichier : on la conserve quand on (re)choisit un fichier.
    const prevLabel = state.config.input.record_label;
    state.inputPath = path;
    state.preview = p;
    state.config.input = {
      path, delimiter: p.delimiter, encoding: p.encoding,
      pid_column: p.suggested_pid_column != null ? p.headers[p.suggested_pid_column] : "",
      record_label: prevLabel,
    };
    // Mapping par défaut : toutes les colonnes d'entrée + existe/CTC-FR ; les
    // autres champs Peppol démarrent dans la drop zone de l'étape 2.
    // Préserve un mapping personnalisé quand on re-choisit le même fichier :
    // on ne le reconstruit que si aucune colonne n'existe encore, ou si les
    // entêtes du nouveau fichier diffèrent de celles de l'ancien preview.
    const headersChanged = !prevHeaders
      || prevHeaders.length !== p.headers.length
      || prevHeaders.some((name, i) => name !== p.headers[i]);
    // Signature identique : la désignation existante prime sur la suggestion —
    // symétrique de la conservation du mapping (et du contexte profil).
    if (!headersChanged && prevPid) state.config.input.pid_column = prevPid;
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
    // Le contexte profil ne survit pas à un changement de signature de
    // colonnes : le profil chargé ne décrit plus ce fichier.
    if (state.profile && headersChanged) state.profile = null;
    // output.dir vide = « dossier du fichier d'entrée » (résolu côté Rust) :
    // pas de valeur à poser ici, le réglage persiste d'un fichier à l'autre.
    renderFilePanel();
    renderProfileBar();
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
  // Un profil sans désignation serait invalide : « Enregistrer sous… » grisé.
  $("btn-saveas-cfg").disabled = !state.config.input.pid_column;
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
const ddz = $("dir-dropzone");
dz.addEventListener("dragover", (e) => { e.preventDefault(); dz.classList.add("over"); });
dz.addEventListener("dragleave", () => dz.classList.remove("over"));
// Le drop de fichier natif arrive par l'événement Tauri drag-drop. Deux cibles
// dans l'étape Fichiers : on route selon la position (px physiques → CSS).
listen("tauri://drag-drop", (e) => {
  dz.classList.remove("over");
  ddz.classList.remove("over");
  $("ppf-dropzone").classList.remove("over");
  const paths = e.payload.paths || [];
  if (!paths.length || STEPS[current] !== "file") return;
  const pos = e.payload.position || { x: 0, y: 0 };
  const dpr = window.devicePixelRatio || 1;
  const x = pos.x / dpr, y = pos.y / dpr;
  const inside = (el) => {
    const r = el.getBoundingClientRect();
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  };
  const csvOk = () => {
    if (/\.(csv|txt)$/i.test(paths[0])) return true;
    banner("warn", `Ce fichier n'est pas un CSV (.csv ou .txt attendu) : ${paths[0]}`);
    return false;
  };
  if (inside(ddz)) {
    if (csvOk()) loadDirectory("file", paths[0]);
  } else if (inside($("ppf-dropzone"))) {
    if (csvOk()) loadPpf(paths[0]);
  } else {
    pickInput(paths[0]);
  }
});

// --- Annuaire Peppol (référence déclarative, onglet Fichiers) ---------------

/** Rend la ligne d'état à partir d'un DirStatus (ou null = jamais chargé).
 *  Données via textContent uniquement (le compteur vient du backend, mais on
 *  ne fait jamais confiance à une entrée dérivée d'un CSV). */
function renderDirStatus(st) {
  const el = $("dir-status");
  el.textContent = "";
  if (!st) {
    el.className = "muted empty";
    el.append(
      h("b", {}, "Jamais chargé."),
      " Téléchargez l'annuaire ou déposez le CSV pour peupler la base."
    );
    return;
  }
  const when = new Date(st.loaded_at * 1000).toLocaleString("fr-FR", {
    day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit",
  });
  const origine = st.source === "download" ? "téléchargé" : "depuis le fichier";
  el.className = "muted";
  el.append(
    h("span", { class: "dot" }, "●"),
    " Dernier chargement : ",
    h("b", {}, when),
    " — ",
    h("b", {}, st.count.toLocaleString("fr-FR")),
    ` adressages 0225 (${origine}).`
  );
}

/** Active/désactive les contrôles et affiche/masque la barre de progression. */
function setDirBusy(busy) {
  $("dir-browse").disabled = busy;
  $("dir-download").disabled = busy;
  $("dir-prog").classList.toggle("hidden", !busy);
  if (!busy) {
    $("dir-bar").classList.remove("indet");
    $("dir-bar").firstElementChild.style.width = "0";
  }
}

let dirBusy = false;

async function loadDirectory(kind, arg) {
  if (dirBusy) return;            // garde anti-concurrence (drop pendant un chargement)
  dirBusy = true;
  setDirBusy(true);
  $("dir-status").classList.add("hidden");
  try {
    const r = kind === "download"
      ? await invoke("download_directory")
      : await invoke("load_directory_file", { path: arg });
    renderDirStatus({ loaded_at: r.loaded_at, count: r.count, source: kind === "download" ? "download" : "file" });
  } catch (err) {
    banner("error", `Annuaire Peppol : ${err}`);
  } finally {
    dirBusy = false;
    setDirBusy(false);
    $("dir-status").classList.remove("hidden");
  }
}

// Progression : phase "download" (octets, barre en %) puis "parse" (lignes, indéterminée).
listen("directory://progress", (e) => {
  const { phase, done, total } = e.payload;
  const bar = $("dir-bar");
  if (phase === "download") {
    bar.classList.remove("indet");
    const mo = (n) => (n / 1048576).toFixed(0);
    if (total) {
      const pct = Math.round((done / total) * 100);
      bar.firstElementChild.style.width = pct + "%";
      $("dir-prog-text").textContent = "Téléchargement de l'annuaire…";
      $("dir-prog-num").textContent = `${mo(done)} Mo / ${mo(total)} Mo · ${pct} %`;
    } else {
      bar.classList.add("indet");
      bar.firstElementChild.style.width = "";
      $("dir-prog-text").textContent = "Téléchargement de l'annuaire…";
      $("dir-prog-num").textContent = `${mo(done)} Mo`;
    }
  } else {
    bar.classList.add("indet");
    bar.firstElementChild.style.width = "";
    $("dir-prog-text").textContent = "Analyse et chargement en base…";
    $("dir-prog-num").textContent = `${done.toLocaleString("fr-FR")} lignes lues`;
  }
});

$("dir-browse").addEventListener("click", async (e) => {
  const btn = e.currentTarget;
  btn.disabled = true; // garde de ré-entrance pendant le dialog (cf. btn-browse)
  try {
    const f = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "txt"] }] });
    if (f) await loadDirectory("file", f);
  } finally {
    btn.disabled = false;
  }
});
$("dir-download").addEventListener("click", () => loadDirectory("download"));

ddz.addEventListener("dragover", (e) => { e.preventDefault(); ddz.classList.add("over"); });
ddz.addEventListener("dragleave", () => ddz.classList.remove("over"));

// Statut initial au démarrage.
invoke("directory_status").then(renderDirStatus).catch(() => {});

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
  c.ppf.active_motifs = $("ppf-motifs").value.trim();
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
  $("ppf-motifs").value = c.ppf.active_motifs;
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
// Libellé « ce que représente une ligne » (record_label) : pluriel affiché là
// où figurait « lignes » — tuiles de bilan (.rec-label) et infobulle
// (data-rec-title, gabarit avec {rec}). Doit rester aligné avec
// RecordLabel::plural() de config.rs.
const RECORD_LABELS = { cf: "CF", client: "clients", utilisateur: "utilisateurs",
                        ligne: "lignes", record: "records" };
function applyRecordLabel() {
  const pl = RECORD_LABELS[state.config.input.record_label] ?? "records";
  document.querySelectorAll(".rec-label").forEach((el) => { el.textContent = pl; });
  document.querySelectorAll("[data-rec-title]").forEach(
    (el) => { el.title = el.dataset.recTitle.replace("{rec}", pl); });
  $("record-label").value = state.config.input.record_label;
}

function fillOutFormat() {
  $("out-encoding").value = state.config.output.encoding;
  $("out-sep").value = state.config.output.separator;
  applyRecordLabel();
}
$("out-encoding").addEventListener("change", (e) => { state.config.output.encoding = e.target.value; renderProfileBar(); });
$("out-sep").addEventListener("change", (e) => { state.config.output.separator = e.target.value; renderProfileBar(); });
$("record-label").addEventListener("change", (e) => {
  state.config.input.record_label = e.target.value;
  applyRecordLabel();
  renderProfileBar();
});

// --- Réglages : persistance (superpopaul.yaml, dossier données de l'app) -----------
/** La tranche de l'état qui va dans le fichier de réglages : API + forme de la
 *  sortie. Ni le fichier d'entrée ni les colonnes (ça, c'est le profil). */
function currentSettings() {
  const c = state.config;
  const { dir, suffix, timestamp_suffix } = c.output;
  return { version: 1, api: c.api,
           output: { dir, suffix, timestamp_suffix },
           ppf: { active_motifs: c.ppf.active_motifs } };
}
/** Fusion sur les défauts de l'état : les champs à leur valeur par défaut sont
 *  absents du YAML (serde skip_serializing_if), un remplacement les perdrait. */
function applySettings(s) {
  Object.assign(state.config.api, s.api);
  Object.assign(state.config.output, s.output);
  Object.assign(state.config.ppf, s.ppf);
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
  applyRecordLabel();
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

/** Empreinte de l'état que porte un profil. La référence (`state.profile.ref`)
 *  est prise au chargement et à chaque enregistrement réussi ; « modifié » =
 *  divergence par comparaison — aucun point de mutation à instrumenter. */
function profileSnapshot() {
  const c = state.config;
  return JSON.stringify({ pid: c.input.pid_column, columns: c.output.columns,
                          encoding: c.output.encoding, separator: c.output.separator,
                          recordLabel: c.input.record_label });
}

/** Le payload envoyé à save_profile — partagé par Enregistrer et
 *  Enregistrer sous… (la validation vit côté Rust, Profile::validate). */
function currentProfilePayload() {
  return {
    version: 1,
    input: { pid_column: state.config.input.pid_column,
             columns_hash: state.preview.columns_hash,
             record_label: state.config.input.record_label },
    output: { encoding: state.config.output.encoding,
              separator: state.config.output.separator },
    columns: state.config.output.columns,
  };
}

/** Barre Format : nom du profil courant, « • modifié » si l'état diverge de
 *  l'instantané, grisage de 💾 (profil courant ET modifié requis). */
function renderProfileBar() {
  const el = $("profile-name");
  const p = state.profile;
  const dirty = p ? profileSnapshot() !== p.ref : false;
  el.replaceChildren();
  if (p) {
    el.append(p.name + " ");
    if (dirty) el.append(h("span", { class: "profile-dirty" }, "• modifié"));
  }
  $("btn-save-cfg").disabled = !(p && dirty);
}
// Hook optionnel appelé par columns.js après chaque rendu du tableau (drag,
// double-clic…) — même motif que window.updateRunModeHint (cockpit.js).
window.updateProfileBar = renderProfileBar;

$("btn-saveas-cfg").addEventListener("click", async () => {
  const dflt = await profileDialogDefault();
  // Propose le nom du profil courant comme point de départ (dans le dossier
  // portable le cas échéant).
  if (state.profile)
    dflt.defaultPath = dflt.defaultPath
      ? `${dflt.defaultPath}/${state.profile.name}` : state.profile.name;
  const f = await save({ filters: [{ name: "YAML", extensions: ["yaml", "yml"] }], ...dflt });
  if (!f) return;
  // Payload et instantané capturés AVANT l'await : une mutation pendant
  // l'aller-retour IPC ne doit pas être marquée « enregistrée » à tort.
  const payload = currentProfilePayload();
  const ref = profileSnapshot();
  try {
    await invoke("save_profile", { path: f, profile: payload });
    state.profile = { path: f, name: f.split(/[\\/]/).pop() ?? f, ref };
    hideBanner();
  } catch (e) {
    banner("error", `${e}`);
  }
  renderProfileBar();
});

$("btn-save-cfg").addEventListener("click", async () => {
  const payload = currentProfilePayload();
  const ref = profileSnapshot();
  try {
    await invoke("save_profile", { path: state.profile.path, profile: payload });
    state.profile.ref = ref;
    hideBanner();
  } catch (e) {
    banner("error", `${e}`);
  }
  renderProfileBar();
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
  state.config.input.record_label = p.input.record_label;
  state.config.output.columns = p.columns;
  state.config.output.encoding = p.output.encoding;
  state.config.output.separator = p.output.separator;
  state.profile = { path: f, name: f.split(/[\\/]/).pop() ?? f, ref: profileSnapshot() };
  hideBanner();
  renderPidSelect();
  fillOutFormat();
  renderOutPreview();
  highlightPidColumn();
  syncStepperGating();
  renderProfileBar();
});

// --- Annuaire PPF (cumulatif, historique des fichiers, onglet Fichiers) -----

/** Recharge le résumé + la table d'historique (via h(), jamais innerHTML). */
function renderPpf() {
  Promise.all([invoke("ppf_summary"), invoke("ppf_files")])
    .then(([sum, files]) => {
      const summary = $("ppf-summary");
      const table = $("ppf-files");
      if (!files.length) {
        summary.className = "muted";
        summary.replaceChildren(document.createTextNode("Aucun fichier chargé."));
        table.classList.add("hidden");
        table.replaceChildren();
        return;
      }
      const plur = sum.file_count > 1 ? "s" : "";
      summary.className = "";
      summary.replaceChildren(
        h("span", { class: "dot" }, "●"),
        " ",
        h("b", {}, sum.distinct_addr.toLocaleString("fr-FR")),
        " adressages en table · ",
        h("b", {}, String(sum.file_count)),
        ` fichier${plur} ingéré${plur}`
      );
      const thead = h("thead", {}, h("tr", {},
        h("th", {}, "Fichier"),
        h("th", { class: "num" }, "Lignes"),
        h("th", { class: "num" }, "Adressages uniques"),
        h("th", { class: "num" }, "Ajoutés"),
        h("th", {}, "Chargé le")
      ));
      const tbody = h("tbody", {});
      for (const f of files) {
        const inner = h("div", { class: "name-inner", title: f.file_name },
          h("span", { class: "fname" }, f.file_name));
        if (f.is_duplicate) inner.append(h("span", { class: "ppf-dup" }, "(doublon)"));
        const name = h("td", { class: "name" }, inner);
        const added = h("td", { class: `num added ${f.added_addr > 0 ? "pos" : "zero"}` });
        if (f.added_addr > 0) added.append(h("b", {}, f.added_addr.toLocaleString("fr-FR")));
        else added.append("0");
        const when = new Date(f.loaded_at * 1000).toLocaleString("fr-FR", {
          day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit",
        });
        tbody.append(h("tr", {},
          name,
          h("td", { class: "num" }, f.lines.toLocaleString("fr-FR")),
          h("td", { class: "num" }, f.unique_addr.toLocaleString("fr-FR")),
          added,
          h("td", { class: "when" }, when)
        ));
      }
      table.replaceChildren(thead, tbody);
      table.classList.remove("hidden");
    })
    .catch((err) => banner("error", `Annuaire PPF : ${err}`));
}

function setPpfBusy(busy) {
  $("ppf-browse").disabled = busy;
  $("ppf-reset").disabled = busy;
  $("ppf-prog").classList.toggle("hidden", !busy);
  if (!busy) {
    $("ppf-bar").classList.remove("indet");
    $("ppf-bar").firstElementChild.style.width = "0";
  }
}

let ppfBusy = false;

async function loadPpf(path) {
  if (ppfBusy) return;
  ppfBusy = true;
  setPpfBusy(true);
  try {
    await invoke("load_ppf_file", { path });
    renderPpf();
  } catch (err) {
    banner("error", `Annuaire PPF : ${err}`);
  } finally {
    ppfBusy = false;
    setPpfBusy(false);
  }
}

// Progression : phase parse uniquement (barre indéterminée, lignes lues).
listen("ppf://progress", (e) => {
  const bar = $("ppf-bar");
  bar.classList.add("indet");
  bar.firstElementChild.style.width = "";
  $("ppf-prog-text").textContent = "Analyse et chargement en base…";
  $("ppf-prog-num").textContent = `${e.payload.done.toLocaleString("fr-FR")} lignes lues`;
});

$("ppf-browse").addEventListener("click", async (e) => {
  const btn = e.currentTarget;
  btn.disabled = true; // garde de ré-entrance pendant le dialog
  try {
    const f = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "txt"] }] });
    if (f) await loadPpf(f);
  } finally {
    btn.disabled = false;
  }
});

// Reset : modale de confirmation maison (nœuds DOM, jamais innerHTML).
$("ppf-reset").addEventListener("click", () => {
  invoke("ppf_summary").then((sum) => {
    modal(
      h("h3", {}, "Vider l'annuaire PPF ?"),
      h("p", { class: "muted" },
        "Cette action supprime les ",
        h("b", {}, sum.distinct_addr.toLocaleString("fr-FR")),
        " adressages de la table et l'historique des ",
        h("b", {}, String(sum.file_count)),
        " fichiers ingérés. Les fichiers sur votre disque ne sont pas touchés. Action irréversible."
      ),
      h("div", { class: "modal-btns" },
        h("button", { onclick: closeModal }, "Annuler"),
        h("button", {
          class: "btn-danger",
          onclick: async () => {
            try {
              await invoke("reset_ppf");
              closeModal();
              renderPpf();
            } catch (err) {
              closeModal();
              banner("error", `Annuaire PPF : ${err}`);
            }
          },
        }, "Réinitialiser")
      )
    );
  }).catch((err) => banner("error", `Annuaire PPF : ${err}`));
});

const pdz = $("ppf-dropzone");
pdz.addEventListener("dragover", (e) => { e.preventDefault(); pdz.classList.add("over"); });
pdz.addEventListener("dragleave", () => pdz.classList.remove("over"));

// État initial au démarrage.
renderPpf();
