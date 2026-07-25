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
    input: { path: "", delimiter: ";", encoding: "utf-8", pid_column: "", record_label: "cf",
             cf_column: "", jj_column: "", raison_sociale_column: "" },
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
    // Le mapping du plan (CF/JJ/raison sociale) dépend de la STRUCTURE du
    // fichier : conservé tel quel ici, et remis à zéro plus bas si les entêtes
    // ont changé (les noms de colonnes n'y existeraient plus).
    const prevPlanCols = {
      cf_column: state.config.input.cf_column ?? "",
      jj_column: state.config.input.jj_column ?? "",
      raison_sociale_column: state.config.input.raison_sociale_column ?? "",
    };
    state.inputPath = path;
    state.preview = p;
    state.config.input = {
      path, delimiter: p.delimiter, encoding: p.encoding,
      pid_column: p.suggested_pid_column != null ? p.headers[p.suggested_pid_column] : "",
      record_label: prevLabel,
      ...prevPlanCols,
    };
    // Mapping par défaut : toutes les colonnes d'entrée + nom PA / PPF
    // utilisable / statut CTC ; les autres champs Peppol démarrent dans la
    // drop zone de l'étape 2.
    // Préserve un mapping personnalisé quand on re-choisit le même fichier :
    // on ne le reconstruit que si aucune colonne n'existe encore, ou si les
    // entêtes du nouveau fichier diffèrent de celles de l'ancien preview.
    const headersChanged = !prevHeaders
      || prevHeaders.length !== p.headers.length
      || prevHeaders.some((name, i) => name !== p.headers[i]);
    // Signature identique : la désignation existante prime sur la suggestion —
    // symétrique de la conservation du mapping (et du contexte profil).
    if (!headersChanged && prevPid) state.config.input.pid_column = prevPid;
    // Entêtes différentes : les colonnes du plan ne désignent plus rien.
    if (headersChanged) {
      state.config.input.cf_column = "";
      state.config.input.jj_column = "";
      state.config.input.raison_sociale_column = "";
    }
    if (state.config.output.columns.length === 0 || headersChanged) {
      state.config.output.columns = [
        ...p.headers.map((name) => ({ source: "input", name })),
        { source: "peppol", field: "pa_name" },
        { source: "peppol", field: "ppf_usable" },
        { source: "peppol", field: "ctc_status" },
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
    // Le téléchargement passe par le réseau : si un proxy est configuré, le
    // pousser au backend (download_directory lit state.config) et s'assurer des
    // identifiants, comme le test API / la calibration. Le chargement d'un
    // fichier local n'utilise pas le réseau.
    if (kind === "download" && state.config.api.proxy) {
      await invoke("set_config", { cfg: state.config });
      await ensureProxyCreds();
    }
    const r = kind === "download"
      ? await invoke("download_directory")
      : await invoke("load_directory_file", { path: arg });
    renderDirStatus({ loaded_at: r.loaded_at, count: r.count, source: kind === "download" ? "download" : "file" });
  } catch (err) {
    // Annulation volontaire de la saisie des identifiants proxy : pas d'erreur.
    if (!(err && err.proxyCancelled)) banner("error", `Annuaire Peppol : ${err}`);
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
                          recordLabel: c.input.record_label,
                          cf: c.input.cf_column ?? "", jj: c.input.jj_column ?? "",
                          rs: c.input.raison_sociale_column ?? "" });
}

/** Le payload envoyé à save_profile — partagé par Enregistrer et
 *  Enregistrer sous… (la validation vit côté Rust, Profile::validate). */
function currentProfilePayload() {
  return {
    version: 1,
    input: { pid_column: state.config.input.pid_column,
             columns_hash: state.preview.columns_hash,
             record_label: state.config.input.record_label,
             // Mapping du plan de charge : transporté même sans écran de
             // saisie (il arrive avec l'onglet Plan) — sinon enregistrer un
             // profil effacerait en silence un mapping déjà présent.
             cf_column: state.config.input.cf_column ?? "",
             jj_column: state.config.input.jj_column ?? "",
             raison_sociale_column: state.config.input.raison_sociale_column ?? "" },
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
  state.config.input.cf_column = p.input.cf_column ?? "";
  state.config.input.jj_column = p.input.jj_column ?? "";
  state.config.input.raison_sociale_column = p.input.raison_sociale_column ?? "";
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

// ===== Plan de charge (Runs de Facturation) ==================================
// L'UI n'a AUCUNE logique métier : elle invoque des commandes et affiche des
// résultats. Tous les calculs (entonnoir, rampe, allocation, compatibilité des
// jours de cycle) vivent dans plan.rs / calendrier.rs.
// Aucun innerHTML : tout passe par h() ou textContent — un CSV et un SMP sont
// des entrées non fiables.
const plan = {
  tab: "param",
  runs: [],            // RunParam[] du calendrier importé
  meps: [],            // dates ISO fournies
  paExclues: new Set(),
  apercu: null,        // dernier PlanApercu
  lignes: [],          // récapitulatif (onglet 2)
  sel: new Set(),      // CF sélectionnés
  tri: { col: "mep_id", asc: true },
  filtres: { mep: "", run: "", pa: "", origine: "", etat: "", q: "" },
  genere: false,
};

const PLAN_ETATS = {
  eligible: ["", "éligible"],
  ctc_non_pret: ["stale", "CTC non prêt"],
  ppf_non_utilisable: ["stale", "PPF non utilisable"],
  absent_du_fichier: ["stale", "absent du fichier"],
};

/** Pousse la config courante au backend. Les commandes du plan lisent la
 *  config SERVEUR : sans cet envoi, elles travailleraient sur celle figée à
 *  l'entrée de l'étape Run. */
async function pousserConfig() {
  try { await invoke("set_config", { cfg: state.config }); }
  catch (e) { planBanner("error", String(e)); }
}

function planBanner(kind, texte) {
  const el = $("plan-banner");
  if (!kind) { el.className = "hidden"; el.replaceChildren(); return; }
  el.className = kind;
  el.replaceChildren(texte);
}

function fmtN(n) { return (n ?? 0).toLocaleString("fr-FR"); }

/** Paramètres envoyés au moteur. Forme exacte de PlanParams (plan.rs). */
function planParams() {
  const forme = $("plan-forme")?.value ?? "plate";
  const pilote = $("plan-pilote")?.checked
    ? { runs: +$("plan-pilote-runs").value || 0, cf_par_run: +$("plan-pilote-cf").value || 0 }
    : null;
  const rampe = { forme, pilote };
  if (forme === "geometrique") rampe.raison = +$("plan-raison").value || 2;
  if (forme === "manuelle") rampe.volumes = {};
  const cibleBrute = $("plan-cible")?.value ?? "";
  return {
    runs: plan.runs,
    debut: $("plan-debut")?.value ?? "",
    fin: $("plan-fin")?.value ?? "",
    meps: plan.meps,
    mep_count: +$("plan-mepcount")?.value || 0,
    cible: cibleBrute === "" ? null : Math.max(0, +cibleBrute | 0),
    seed: +$("plan-seed")?.value || 0,
    pa_exclues: [...plan.paExclues],
    rampe,
  };
}

// --- Panneau latéral ---------------------------------------------------------
function renderPlanAside() {
  const cfg = state.config.input;
  const entetes = state.preview?.headers ?? [];
  const manque = !cfg.cf_column || !cfg.jj_column;

  // `champ` est le nom EXACT du champ d'InputConfig : le mapping est poussé au
  // backend dans la foulée, sinon le calcul se ferait sur la config figée à
  // l'entrée de l'étape Run et se plaindrait de colonnes « non désignées »
  // alors qu'elles sont à l'écran.
  const selCol = (id, champ, val, avecVide) => h("select", {
    id, onchange: async (e) => {
      state.config.input[champ] = e.target.value;
      await pousserConfig();
      renderPlanAside(); planRecalc();
    },
  },
    ...(avecVide ? [h("option", { value: "" }, "(aucune)")] : [h("option", { value: "" }, "— choisir —")]),
    ...entetes.map((x) => {
      const o = h("option", { value: x }, x);
      if (x === val) o.selected = true;
      return o;
    }));

  const bloc = h("div", { id: "plan-cols", class: manque ? "need" : "" },
    h("label", {}, "Compte de facturation (CF)"),
    selCol("plan-col-cf", "cf_column", cfg.cf_column, false),
    h("label", {}, "Jour de cycle (JJ)"),
    selCol("plan-col-jj", "jj_column", cfg.jj_column, false),
    h("label", {}, "Raison sociale ", h("span", { class: "muted" }, "— information")),
    selCol("plan-col-rs", "raison_sociale_column", cfg.raison_sociale_column, true),
    h("p", { class: "field-hint" }, "Modifiable ici comme à l'étape Format : c'est le même réglage."));

  const chips = h("div", { class: "chips" },
    ...plan.meps.map((m) => h("span", { class: "chip" }, m,
      h("button", { class: "btn-ghost", title: "Retirer cette MEP",
        onclick: () => { plan.meps = plan.meps.filter((x) => x !== m); renderPlanAside(); planRecalc(); } }, "✕"))));

  const forme = $("plan-forme")?.value ?? "lineaire";
  const pilOn = $("plan-pilote")?.checked ?? false;

  $("plan-aside").replaceChildren(
    h("h3", { id: "plan-cols-title", class: manque ? "need" : "" }, "Colonnes"), bloc,

    h("h3", {}, "Calendrier de facturation"),
    h("button", { onclick: importerRuns }, "Charger runs.csv…"),
    h("p", { class: "field-hint" }, plan.runs.length
      ? `✓ ${plan.runs.length} Runs de Facturation chargés.`
      : "En-tête attendu DATE_RUN;NUM_RUN;JJS — date en JJ/MM/AAAA, jours séparés par des tirets."),

    h("h3", {}, "Fenêtre FUT"),
    h("div", { class: "row" },
      h("label", {}, "Début", h("input", { type: "date", id: "plan-debut", oninput: planRecalc })),
      h("label", {}, "Fin", h("input", { type: "date", id: "plan-fin", oninput: planRecalc }))),

    h("h3", {}, "Mises en production"),
    chips,
    h("label", {}, "Ajouter une MEP"),
    h("input", { type: "date", id: "plan-mepadd", onchange: (e) => {
      const v = e.target.value;
      if (v && !plan.meps.includes(v)) { plan.meps.push(v); plan.meps.sort(); }
      e.target.value = ""; renderPlanAside(); planRecalc();
    } }),
    h("label", {}, "Nombre total visé"),
    h("input", { type: "number", id: "plan-mepcount", min: "0", value: "0", style: "width:80px", oninput: planRecalc }),

    h("h3", {}, "Cible"),
    h("label", {}, "Comptes distincts à traiter"),
    h("input", { type: "number", id: "plan-cible", min: "1", placeholder: "auto", style: "width:120px", oninput: planRecalc }),
    h("p", { class: "field-hint" }, "Vide = tout le pool éligible atteignable."),

    h("h3", {}, "Rampe de montée en charge"),
    h("label", {}, "Forme"),
    h("select", { id: "plan-forme", onchange: () => { renderPlanAside(); planRecalc(); } },
      ...[["plate", "Plate (équirépartie)"], ["lineaire", "Linéaire (croissance douce)"],
          ["geometrique", "Géométrique (raison réglable)"]].map(([v, t]) => {
        const o = h("option", { value: v }, t);
        if (v === forme) o.selected = true;
        return o;
      })),
    ...(forme === "geometrique"
      ? [h("label", {}, "Raison"), h("input", { type: "number", id: "plan-raison", min: "1.1", step: "0.05", value: "1.55", style: "width:90px", oninput: planRecalc })]
      : []),
    h("label", {}, h("input", { type: "checkbox", id: "plan-pilote", onchange: () => { renderPlanAside(); planRecalc(); } }), " Pilote prudent au démarrage"),
    ...(pilOn
      ? [h("label", {}, "Durée du pilote (runs)"), h("input", { type: "number", id: "plan-pilote-runs", min: "0", value: "0", style: "width:80px", oninput: planRecalc }),
         h("label", {}, "Comptes par run de pilote"), h("input", { type: "number", id: "plan-pilote-cf", min: "0", value: "0", style: "width:80px", oninput: planRecalc }),
         h("p", { class: "field-hint" }, "Le niveau du pilote sert de socle : la rampe ne redescend jamais en dessous.")]
      : []),

    h("h3", {}, "Options"),
    h("label", {}, "Seed"),
    h("input", { type: "number", id: "plan-seed", value: "42", style: "width:100px", oninput: planRecalc }),
    h("p", { class: "field-hint" }, "Départage déterministe à priorité égale. Même seed, même plan."),

    h("button", { id: "btn-plan-gen", class: "btn-primary", style: "width:100%;margin-top:16px", onclick: genererPlan },
      plan.genere ? "Régénérer le plan" : "Générer le plan"),
    h("p", { class: "field-hint" }, "Conservés à l'identique : MEP gelées, retouches manuelles et comptes retirés."),
  );
}

async function importerRuns() {
  const chemin = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "txt"] }] });
  if (!chemin) return;
  try {
    const r = await invoke("plan_import_runs", { path: chemin });
    plan.runs = r.runs;
    planBanner(r.erreurs.length ? "warn" : null,
      r.erreurs.length ? `${r.erreurs.length} ligne(s) en erreur : ${r.erreurs.slice(0, 3).join(" · ")}` : "");
    renderPlanAside();
    planRecalc();
  } catch (e) { planBanner("error", String(e)); }
}

// --- Onglet 1 : paramétrage --------------------------------------------------
function marche(lbl, val, base, precedent, final) {
  const pct = base > 0 ? Math.max(0, Math.min(100, (val / base) * 100)) : 0;
  const perte = precedent == null ? "" : `−${fmtN(precedent - val)}`;
  return h("div", { class: final ? "fstep final" : "fstep" },
    h("span", { class: "lbl" }, lbl),
    h("span", { class: "bar" }, h("span", { style: `width:${pct.toFixed(1)}%` })),
    h("span", { class: "val" }, fmtN(val)),
    h("span", { class: "loss" }, perte));
}

function renderPlanParam() {
  const box = $("plan-param");
  const a = plan.apercu;
  if (!a) {
    box.replaceChildren(h("p", { class: "muted" },
      plan.runs.length
        ? "Renseigne la fenêtre FUT et au moins une MEP pour calculer le plan."
        : "Charge le calendrier des Runs de Facturation pour commencer."));
    return;
  }
  const f = a.funnel;
  const b = f.lignes || 1;
  const noeuds = [];

  noeuds.push(h("h2", {}, "Éligibilité"));
  noeuds.push(h("p", { class: "field-hint" },
    "Un compte est éligible si son statut CTC est prêt et qu'il est PPF utilisable (motif actif ET PDP réelle sur la même ligne)."));
  noeuds.push(h("div", {},
    marche("Lignes du fichier", f.lignes, b, null),
    marche("Comptes distincts", f.cf_distincts, b, f.lignes),
    marche("Jour de cycle valide", f.jj_valide, b, f.cf_distincts),
    marche("Adressage résolu", f.resolus, b, f.jj_valide),
    marche("Statut CTC « prêt »", f.ctc_ready, b, f.resolus),
    marche("PPF utilisable", f.ppf_usable, b, f.ctc_ready),
    marche("COMPTES ÉLIGIBLES", f.eligibles, b, f.ppf_usable, true)));

  noeuds.push(h("h2", {}, "Runs de Facturation"));
  noeuds.push(h("p", { class: "field-hint" },
    `${a.details.length} run(s) retenu(s) · rattachement à la dernière MEP strictement antérieure.`));
  const tbl = h("table", { class: "plan-data" },
    h("tr", {}, ...["Run", "Date", "JJ facturés", "MEP", "Visé", "Report", "Stock JJ", "Placé", "Reliquat"]
      .map((t, i) => h("th", { class: i >= 4 ? "n" : "" }, t))));
  for (const d of a.details) {
    tbl.append(h("tr", {},
      h("td", {}, d.run_num),
      h("td", {}, d.run_date),
      h("td", { class: "jj" }, d.jjs.join(", ")),
      h("td", {}, `${d.mep_id} (${d.mep_date})`),
      h("td", { class: "n" }, fmtN(d.vise)),
      h("td", { class: d.report_entrant ? "n carry" : "n zero" }, d.report_entrant ? `+${fmtN(d.report_entrant)}` : "—"),
      h("td", { class: "n" }, fmtN(d.stock)),
      h("td", { class: "n" }, fmtN(d.place)),
      h("td", { class: d.reliquat ? "n carry" : "n zero" }, d.reliquat ? `+${fmtN(d.reliquat)}` : "0")));
  }
  noeuds.push(tbl);

  if (a.avertissements.length) {
    const w = h("div", { class: "plan-warns" });
    for (const t of a.avertissements) w.append(h("p", {}, h("span", { class: "ico" }, "⚠ "), t));
    noeuds.push(w);
  }

  noeuds.push(h("h2", {}, "Plateformes"));
  noeuds.push(h("p", { class: "field-hint" },
    "Décocher une plateforme retire ses comptes du pool. Le quota est une cible souple : quand le volume d'un run dépasse les quotas restants des plateformes présentes, le volume prime."));
  noeuds.push(h("div", { class: "pa-line pa-head" },
    h("span", {}), h("span", {}, "Plateforme"), h("span", {}),
    h("span", { class: "n" }, "Éligibles"), h("span", { class: "n" }, "Quota")));
  const maxPa = Math.max(1, ...a.plateformes.map((p) => p.eligibles));
  for (const p of a.plateformes) {
    const off = plan.paExclues.has(p.nom);
    const cb = h("input", { type: "checkbox", onchange: () => {
      if (plan.paExclues.has(p.nom)) plan.paExclues.delete(p.nom); else plan.paExclues.add(p.nom);
      planRecalc();
    } });
    cb.checked = !off;
    noeuds.push(h("div", { class: off ? "pa-line off" : "pa-line" },
      cb, h("span", {}, p.nom),
      h("span", { class: "bar" }, h("span", { style: `width:${((p.eligibles / maxPa) * 100).toFixed(1)}%` })),
      h("span", { class: "n" }, fmtN(p.eligibles)),
      h("span", { class: "n" }, off ? "—" : fmtN(p.quota))));
  }
  // Les plateformes exclues sortent du pool : elles n'apparaissent plus dans
  // l'aperçu. On les rappelle pour pouvoir les réintégrer.
  for (const nom of plan.paExclues) {
    if (a.plateformes.some((p) => p.nom === nom)) continue;
    const cb = h("input", { type: "checkbox", onchange: () => { plan.paExclues.delete(nom); planRecalc(); } });
    noeuds.push(h("div", { class: "pa-line off" }, cb, h("span", {}, nom),
      h("span", {}), h("span", { class: "n" }, "—"), h("span", { class: "n" }, "—")));
  }

  if (plan.genere && plan.fichiers) {
    const r = h("div", { class: "plan-result" },
      h("h3", {}, `Plan enregistré — ${fmtN(a.total)} comptes sur ${a.meps.length} MEP`),
      h("div", { class: "plan-kv" },
        h("span", {}, h("b", {}, fmtN(a.geles)), "gelés"),
        h("span", {}, h("b", {}, fmtN(a.epingles)), "manuels"),
        h("span", {}, h("b", {}, fmtN(a.retires)), "retirés")));
    const ul = h("ul", {});
    for (const fi of plan.fichiers)
      ul.append(h("li", {}, h("code", {}, fi.chemin), ` — ${fmtN(fi.comptes)} comptes`));
    r.append(ul);
    r.append(h("div", { class: "actions" },
      h("button", { onclick: async (ev) => {
        const b = ev.currentTarget, lbl = b.textContent;
        b.disabled = true; b.textContent = "…";
        try {
          const p = await invoke("plan_rapport");
          window.__TAURI__.opener?.revealItemInDir(p);
        } catch (e) { planBanner("error", String(e)); }
        b.disabled = false; b.textContent = lbl;
      } }, "Rapport du plan…")));
    noeuds.push(r);
  }
  box.replaceChildren(...noeuds);
}

// --- Onglet 2 : comptes de facturation ---------------------------------------
function lignesFiltrees() {
  const f = plan.filtres;
  const q = f.q.trim().toLowerCase();
  let out = plan.lignes.filter((l) =>
    (!f.mep || String(l.mep_id) === f.mep)
    && (!f.run || l.run_num === f.run)
    && (!f.pa || l.pa === f.pa)
    && (!f.origine || l.origine === f.origine)
    && (!f.etat || (f.etat === "retire" ? l.retire_motif != null : l.etat === f.etat))
    && (!q || [l.cf, l.participant, l.raison_sociale].some((x) => (x ?? "").toLowerCase().includes(q))));
  const { col, asc } = plan.tri;
  out = out.slice().sort((a, b) => {
    const va = a[col], vb = b[col];
    const c = typeof va === "number" ? va - vb : String(va).localeCompare(String(vb), "fr");
    return asc ? c : -c;
  });
  return out;
}

function renderPlanRecap() {
  const box = $("plan-recap");
  if (!plan.lignes.length) {
    box.replaceChildren(h("p", { class: "muted" }, "Aucun plan enregistré. Génère-le depuis l'onglet Paramétrage."));
    return;
  }
  const uniques = (k) => [...new Set(plan.lignes.map((l) => l[k]))].sort();
  const selectFiltre = (cle, libelle, valeurs) => h("select", {
    onchange: (e) => { plan.filtres[cle] = e.target.value; renderPlanRecap(); },
  }, h("option", { value: "" }, libelle),
     ...valeurs.map((v) => { const o = h("option", { value: String(v) }, String(v));
       if (String(v) === plan.filtres[cle]) o.selected = true; return o; }));

  const barre = h("div", { class: "plan-toolbar" },
    selectFiltre("mep", "Toutes les MEP", uniques("mep_id")),
    selectFiltre("run", "Tous les runs", uniques("run_num")),
    selectFiltre("pa", "Toutes les plateformes", uniques("pa")),
    selectFiltre("origine", "Toutes origines", ["auto", "couverture", "manuel"]),
    selectFiltre("etat", "Tous états", ["eligible", "ctc_non_pret", "ppf_non_utilisable", "absent_du_fichier", "retire"]),
    h("input", { class: "grow", type: "search", placeholder: "Rechercher un compte, un adressage, une raison sociale…",
      oninput: (e) => { plan.filtres.q = e.target.value; renderPlanRecap(); } }));

  const visibles = lignesFiltrees();
  const actives = plan.lignes.filter((l) => l.retire_motif == null).length;
  const noeuds = [barre];

  if (plan.sel.size) {
    noeuds.push(h("div", { class: "plan-selbar" },
      h("span", {}, h("b", {}, String(plan.sel.size)), " compte(s) sélectionné(s)"),
      h("span", { class: "spacer" }),
      h("button", { onclick: ouvrirDeplacer }, "Déplacer vers un run…"),
      h("button", { class: "btn-danger", onclick: ouvrirRetrait }, "Retirer…"),
      h("button", { class: "btn-ghost", onclick: () => { plan.sel.clear(); renderPlanRecap(); } }, "Tout désélectionner")));
  }
  noeuds.push(h("div", { class: "plan-toolbar" },
    h("button", { class: "btn-primary", onclick: ouvrirAjout }, "+ Ajouter des comptes…"),
    h("span", { class: "grow" }),
    h("span", { class: "muted", style: "font-size:12.5px" },
      `${fmtN(actives)} ligne(s) active(s) · ${fmtN(plan.lignes.length - actives)} retirée(s) · ${fmtN(visibles.length)} affichée(s)`)));

  const entete = h("tr", {}, h("th", {}, ""));
  for (const [cle, lbl] of [["cf", "Compte"], ["participant", "Adressage"], ["raison_sociale", "Raison sociale"],
                            ["jj", "JJ"], ["pa", "Plateforme"], ["mep_id", "MEP"], ["run_num", "Run"],
                            ["origine", "Origine"], ["etat", "État"]]) {
    const th = h("th", { class: plan.tri.col === cle ? "sortable sorted" : "sortable",
      onclick: () => {
        if (plan.tri.col === cle) plan.tri.asc = !plan.tri.asc;
        else plan.tri = { col: cle, asc: true };
        renderPlanRecap();
      } }, lbl + (plan.tri.col === cle ? (plan.tri.asc ? " ▲" : " ▼") : ""));
    entete.append(th);
  }
  const tbl = h("table", { class: "plan-data" }, entete);
  // Plafond d'affichage : au-delà, le DOM devient le goulot. Le nombre
  // masqué est dit explicitement — jamais de troncature muette.
  const PLAFOND = 500;
  for (const l of visibles.slice(0, PLAFOND)) {
    const cb = h("input", { type: "checkbox", onchange: () => {
      if (plan.sel.has(l.cf)) plan.sel.delete(l.cf); else plan.sel.add(l.cf);
      renderPlanRecap();
    } });
    cb.checked = plan.sel.has(l.cf);
    const retire = l.retire_motif != null;
    const [cls, txt] = PLAN_ETATS[l.etat] ?? ["", l.etat];
    const etatNode = retire
      ? h("span", {}, h("span", { class: "tag removed" }, "retiré"),
          h("div", { class: "motif" }, l.retire_motif))
      : (cls ? h("span", { class: `tag ${cls}` }, txt) : h("span", { class: "muted" }, txt));
    const orig = l.origine === "manuel" ? h("span", { class: "tag pinned" }, "📌 manuelle")
      : l.origine === "couverture" ? h("span", { class: "tag fill" }, "couverture")
      : h("span", { class: "tag" }, "allouée");
    tbl.append(h("tr", { class: [retire ? "removed" : "", plan.sel.has(l.cf) ? "sel" : ""].join(" ").trim() },
      h("td", {}, cb),
      h("td", { class: "cf" }, l.cf),
      h("td", { class: "addr" }, l.participant),
      h("td", {}, l.raison_sociale),
      h("td", { class: "n" }, String(l.jj)),
      h("td", {}, l.pa),
      h("td", {}, `${l.mep_id}`, l.gelee ? " " : "", l.gelee ? h("span", { class: "tag frozen" }, "❄ gelé") : ""),
      h("td", {}, l.run_num),
      h("td", {}, orig),
      h("td", {}, etatNode)));
  }
  noeuds.push(tbl);
  if (visibles.length > PLAFOND)
    noeuds.push(h("p", { class: "muted", style: "text-align:center;font-size:12.5px" },
      `… ${fmtN(visibles.length - PLAFOND)} ligne(s) supplémentaire(s) non affichée(s) — affine les filtres.`));
  box.replaceChildren(...noeuds);
}

// --- Modales de retouche -----------------------------------------------------
async function ouvrirDeplacer() {
  const cfs = [...plan.sel];
  // Un compte ne peut aller que sur un run couvrant SON jour de cycle. On ne
  // propose donc que l'intersection des runs compatibles de la sélection —
  // proposer l'impossible pour le refuser ensuite serait hostile.
  let communs = null;
  try {
    for (const cf of cfs) {
      const l = plan.lignes.find((x) => x.cf === cf);
      const rs = await invoke("plan_runs_compatibles", { jj: l.jj });
      communs = communs === null ? new Set(rs) : new Set(rs.filter((r) => communs.has(r)));
    }
  } catch (e) { return planBanner("error", String(e)); }
  if (!communs || communs.size === 0) {
    return planBanner("warn",
      "Aucun Run de Facturation ne couvre à la fois tous les jours de cycle sélectionnés.");
  }
  const sel = h("select", {}, ...[...communs].sort().map((r) => h("option", { value: r }, r)));
  modal(
    h("h3", {}, `Déplacer ${cfs.length} compte(s)`),
    h("p", { class: "field-hint" }, "Seuls les runs couvrant les jours de cycle sélectionnés sont proposés."),
    h("p", {}, h("label", {}, "Run de Facturation "), sel),
    h("div", { class: "actions" },
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"),
      h("button", { class: "btn-primary", onclick: async () => {
        try {
          await invoke("plan_deplacer", { cfs, runNum: sel.value });
          closeModal(); plan.sel.clear(); await rechargerRecap();
        } catch (e) { planBanner("error", String(e)); closeModal(); }
      } }, "Déplacer")));
}

function ouvrirRetrait() {
  const cfs = [...plan.sel];
  const geles = plan.lignes.filter((l) => cfs.includes(l.cf) && l.gelee);
  const zone = h("textarea", { rows: "3", style: "width:100%",
    placeholder: "Ex. : migration PDP repoussée par le client, compte clôturé, incident connu…" });
  const btn = h("button", { class: "btn-danger", onclick: async () => {
    try {
      await invoke("plan_retirer", { cfs, motif: zone.value });
      closeModal(); plan.sel.clear(); await rechargerRecap();
    } catch (e) { planBanner("error", String(e)); closeModal(); }
  } }, `Retirer ${cfs.length} compte(s)`);
  btn.disabled = true;
  zone.addEventListener("input", () => { btn.disabled = zone.value.trim() === ""; });

  const noeuds = [
    h("h3", {}, `Retirer ${cfs.length} compte(s) du plan`),
    h("p", { class: "field-hint" },
      "Les lignes restent consultables via le filtre « retiré » et ne seront pas replacées par une régénération. Le retrait est annulable."),
  ];
  if (geles.length) {
    // Les fichiers sont cumulatifs : retirer d'une MEP livrée change un
    // fichier déjà transmis. C'est assumé, mais ça se dit au moment de l'acte.
    noeuds.push(h("div", { class: "danger-note" },
      `⚠ ${geles.length} compte(s) appartiennent à une MEP gelée (${[...new Set(geles.map((l) => l.mep_date))].join(", ")}). `
      + "Son fichier a déjà été transmis : il changera au prochain tirage."));
  }
  noeuds.push(h("label", { class: "field-hint" }, "Motif du retrait (obligatoire)"), zone,
    h("div", { class: "actions" }, h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"), btn));
  modal(...noeuds);
}

async function ouvrirAjout() {
  let candidats;
  try { candidats = await invoke("plan_candidats"); }
  catch (e) { return planBanner("error", String(e)); }
  if (!candidats.length) return planBanner("info", "Tous les comptes du fichier sont déjà au plan.");

  const choisis = new Set();
  const recherche = h("input", { type: "search", style: "width:100%",
    placeholder: "Filtrer par compte ou raison sociale…" });
  const liste = h("div", { style: "max-height:260px;overflow:auto;margin:8px 0" });
  const selRun = h("select", {});
  const dessiner = () => {
    const q = recherche.value.trim().toLowerCase();
    liste.replaceChildren(...candidats
      .filter((c) => !q || c.cf.toLowerCase().includes(q) || (c.raison_sociale ?? "").toLowerCase().includes(q))
      .slice(0, 200)
      .map((c) => {
        const cb = h("input", { type: "checkbox", onchange: () => {
          if (choisis.has(c.cf)) choisis.delete(c.cf); else choisis.add(c.cf);
          majRuns();
        } });
        cb.checked = choisis.has(c.cf);
        return h("label", { style: "display:block;font-size:12.5px" }, cb, ` ${c.cf} — ${c.raison_sociale} `,
          h("span", { class: "muted" }, `(JJ ${c.jj}, ${c.pa || "sans plateforme"})`),
          c.eligible ? "" : h("span", { class: "tag stale" }, "non éligible"));
      }));
  };
  const majRuns = async () => {
    selRun.replaceChildren();
    if (!choisis.size) return;
    let communs = null;
    for (const cf of choisis) {
      const c = candidats.find((x) => x.cf === cf);
      const rs = await invoke("plan_runs_compatibles", { jj: c.jj });
      communs = communs === null ? new Set(rs) : new Set(rs.filter((r) => communs.has(r)));
    }
    selRun.replaceChildren(...[...(communs ?? [])].sort().map((r) => h("option", { value: r }, r)));
  };
  recherche.addEventListener("input", dessiner);
  dessiner();

  modal(
    h("h3", {}, "Ajouter des comptes au plan"),
    h("p", { class: "field-hint" },
      "Les comptes non éligibles sont proposés et signalés : les ajouter est un choix assumé. Seuls les runs couvrant les jours de cycle retenus sont ensuite proposés."),
    recherche, liste,
    h("p", {}, h("label", {}, "Run de Facturation "), selRun),
    h("div", { class: "actions" },
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"),
      h("button", { class: "btn-primary", onclick: async () => {
        if (!choisis.size || !selRun.value) return;
        try {
          await invoke("plan_ajouter", { cfs: [...choisis], runNum: selRun.value });
          closeModal(); await rechargerRecap();
        } catch (e) { planBanner("error", String(e)); closeModal(); }
      } }, "Ajouter")));
}

// --- Cycle de vie de l'écran -------------------------------------------------
let planRecalcTimer = null;
/** Recalcul débounce : chaque frappe déclencherait sinon un scan complet. */
function planRecalc() {
  clearTimeout(planRecalcTimer);
  planRecalcTimer = setTimeout(async () => {
    const p = planParams();
    if (!p.runs.length || !p.debut || !p.fin) { plan.apercu = null; renderPlanParam(); return; }
    try {
      plan.apercu = await invoke("plan_preview", { params: p });
      planBanner(null);
    } catch (e) {
      plan.apercu = null;
      planBanner("warn", String(e));
    }
    renderPlanParam();
  }, 250);
}

async function genererPlan() {
  try {
    const r = await invoke("plan_generate", { params: planParams() });
    plan.apercu = r.apercu;
    plan.fichiers = r.fichiers;
    plan.genere = true;
    planBanner(null);
    renderPlanAside();
    renderPlanParam();
    await rechargerRecap();
  } catch (e) { planBanner("error", String(e)); }
}

async function rechargerRecap() {
  try {
    plan.lignes = await invoke("plan_lignes");
    $("plan-tab-count").textContent = plan.lignes.length ? fmtN(plan.lignes.length) : "";
    renderPlanRecap();
  } catch (e) { planBanner("error", String(e)); }
}

function planShowTab(t) {
  plan.tab = t;
  document.querySelectorAll(".plan-tab").forEach((b) => b.classList.toggle("active", b.dataset.ptab === t));
  $("plan-param").classList.toggle("hidden", t !== "param");
  $("plan-recap").classList.toggle("hidden", t !== "recap");
}

async function ouvrirPlan() {
  $("plan-screen").classList.remove("hidden");
  $("plan-sub").textContent = state.inputPath
    ? state.inputPath.split(/[\\/]/).pop() : "";
  // Le mapping a pu changer depuis l'entrée dans l'étape Run (profil chargé,
  // étape Format revisitée) : on resynchronise avant tout calcul.
  await pousserConfig();
  // État persisté : paramètres et calendrier du plan enregistré.
  try {
    const enr = await invoke("plan_load");
    if (enr) {
      plan.runs = enr.params.runs ?? [];
      plan.meps = enr.params.meps ?? [];
      plan.paExclues = new Set(enr.params.pa_exclues ?? []);
      plan.genere = true;
      renderPlanAside();
      if ($("plan-debut")) $("plan-debut").value = enr.params.debut ?? "";
      if ($("plan-fin")) $("plan-fin").value = enr.params.fin ?? "";
      if ($("plan-mepcount")) $("plan-mepcount").value = enr.params.mep_count ?? 0;
      if ($("plan-cible")) $("plan-cible").value = enr.params.cible ?? "";
      if ($("plan-seed")) $("plan-seed").value = enr.params.seed ?? 42;
      $("plan-foot-info").textContent =
        `Plan enregistré depuis ${enr.fichier}` + (enr.autre_fichier ? " — ⚠ le fichier ouvert est différent" : "");
      if (enr.autre_fichier)
        planBanner("warn",
          `Le plan enregistré a été produit depuis « ${enr.fichier} », différent du fichier ouvert : les lignes gelées peuvent ne plus correspondre.`);
    } else {
      renderPlanAside();
    }
  } catch (e) {
    renderPlanAside();
    planBanner("warn", String(e));
  }
  planShowTab("param");
  await rechargerRecap();
  planRecalc();
}

function fermerPlan() { $("plan-screen").classList.add("hidden"); }

$("btn-plan-open").addEventListener("click", ouvrirPlan);
$("btn-plan-close").addEventListener("click", fermerPlan);
$("btn-plan-back").addEventListener("click", fermerPlan);
document.querySelectorAll(".plan-tab").forEach((b) =>
  b.addEventListener("click", () => planShowTab(b.dataset.ptab)));
