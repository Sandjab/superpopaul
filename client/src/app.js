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
  // Mappings de colonnes mémorisés (réglages persistés), retrouvés par
  // signature d'en-têtes — pas par chemin de fichier, qui bouge.
  mappings: [],
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
  // Le contenant est partagé par toutes les modales : les variantes d'une
  // ouverture précédente doivent tomber, sinon la confirmation suivante en
  // hérite. Toute nouvelle variante de `#modal` s'ajoute ici.
  const el = $("modal");
  el.classList.remove("modal-wide", "modal-resolve");
  el.replaceChildren(...nodes);
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
      // …sauf si cette STRUCTURE a déjà été mappée : la signature des en-têtes
      // retrouve la désignation par-delà les relances, sans dépendre du chemin
      // du fichier. Sans cela, tout redémarrage impose de re-désigner de
      // mémoire — et une erreur de désignation vide l'écran de plan en silence.
      const memo = state.mappings.find((m) => m.columns_hash === p.columns_hash);
      if (memo) {
        state.config.input.cf_column = memo.cf_column ?? "";
        state.config.input.jj_column = memo.jj_column ?? "";
        state.config.input.raison_sociale_column = memo.raison_sociale_column ?? "";
        if (memo.pid_column) state.config.input.pid_column = memo.pid_column;
      }
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
  // Mémorisation en arrière-plan : elle ne conditionne rien à l'écran, et une
  // désignation qui attendrait une écriture disque se sentirait.
  return memoriserColonnes();
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
           ppf: { active_motifs: c.ppf.active_motifs },
           mappings: state.mappings };
}

/** Mémorise les colonnes désignées pour la structure du fichier ouvert.
 *
 *  Sans fichier, il n'y a pas de signature : mémoriser attacherait le mapping
 *  à n'importe quelle structure. Le tri (entrée vide, doublon de signature,
 *  liste bornée) appartient au backend — l'UI ne fait que transmettre. */
async function memoriserColonnes() {
  const p = state.preview;
  if (!p?.columns_hash) return;
  const i = state.config.input;
  try {
    const s = await invoke("remember_columns", {
      settings: currentSettings(),
      mapping: {
        columns_hash: p.columns_hash,
        pid_column: i.pid_column ?? "",
        cf_column: i.cf_column ?? "",
        jj_column: i.jj_column ?? "",
        raison_sociale_column: i.raison_sociale_column ?? "",
      },
    });
    state.mappings = s.mappings ?? [];
  } catch (e) {
    // Un mapping non mémorisé se re-désigne ; interrompre l'utilisateur pour
    // ça serait disproportionné.
    console.warn("mapping non mémorisé :", e);
  }
}
/** Fusion sur les défauts de l'état : les champs à leur valeur par défaut sont
 *  absents du YAML (serde skip_serializing_if), un remplacement les perdrait. */
function applySettings(s) {
  state.mappings = s.mappings ?? [];
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
      // Refus du proxy lui-même : re-demander les identifiants au prochain clic.
      if (estRefusDuProxy(err)) proxyCredsGiven = false;
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
      // Refus du proxy lui-même : re-demander les identifiants au prochain clic.
      if (estRefusDuProxy(err)) proxyCredsGiven = false;
      out.textContent = `❌ ${err}`;
    }
  }
}
$("btn-calibrate").addEventListener("click", runCalibration);

/** Vrai pour les seuls messages qui valent une ressaisie des identifiants du
 *  proxy : « … (HTTP 407) » (ApiError::ProxyAuth) et « … (407) »
 *  (DirectClient::preflight_proxy).
 *
 *  Motif étroit à dessein. Chercher « proxy » dans le texte attraperait aussi
 *  le refus venu d'un INTERMÉDIAIRE, dont le message nomme la « page de
 *  confirmation d'un proxy d'entreprise » : aucune ressaisie ne le débloque, et
 *  la modale s'ouvrirait pour rien. Chercher « 407 » nu se déclencherait sur
 *  une URL qui porte ce nombre — l'URL de l'API figure dans le message, et
 *  c'est le piège déjà rencontré côté Rust
 *  (api.rs::url_contenant_407_ne_declenche_pas_proxyauth). */
function estRefusDuProxy(err) {
  return /\((?:HTTP )?407\)/.test(String(err));
}

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

// --- Loupe : résolution d'un adressage unitaire ---------------------------
// Consultation seule : la commande n'écrit rien, et l'écran le rappelle.
$("btn-resolve").addEventListener("click", () => {
  const champ = h("input", {
    type: "text", id: "resolve-input",
    placeholder: "SIREN, 0225:… ou identifiant complet",
  });
  const sortie = h("div", { id: "resolve-result" });
  const go = h("button", {
    id: "resolve-go", class: "btn-primary",
    onclick: () => lancerResolution(champ, sortie, go),
  }, "Résoudre");
  // Le champ est vide à l'ouverture : un bouton d'apparence active qui ne fait
  // rien laisse croire à une panne. C'est la saisie qui l'ouvre.
  go.disabled = true;
  champ.addEventListener("input", () => { go.disabled = !champ.value.trim(); });
  champ.addEventListener("keydown", (ev) => {
    // Même garde que le clic : sans elle, Entrée relance pendant qu'une
    // résolution est en vol, et la réponse la plus lente s'affiche sous la
    // forme canonique de la plus récente.
    if (ev.key === "Enter" && !go.disabled) lancerResolution(champ, sortie, go);
  });
  modal(
    h("div", { class: "modal-h" },
      h("h3", {}, "Résoudre un adressage"),
      h("span", { class: "tag gold" }, "consultation seule"),
      h("button", { class: "btn-ghost", onclick: closeModal }, "✕")),
    h("div", { class: "resolve-saisie" }, champ, go),
    sortie,
    h("p", { class: "resolve-hint" },
      "Le résultat n'est pas enregistré : consulter un compte ici ne le retire "
      + "pas du périmètre d'un run futur."));
  // Contenu le plus haut de l'application : sans plafond de hauteur il déborde
  // par le haut, hors d'atteinte (cf. `#modal.modal-resolve` dans styles.css).
  $("modal").classList.add("modal-resolve");
  champ.focus();
});

async function lancerResolution(champ, sortie, go) {
  const saisi = champ.value.trim();
  if (!saisi) return;
  go.disabled = true;
  sortie.replaceChildren(h("div", { class: "resolve-spin" }, "Résolution en cours…"));
  try {
    const r = await invoke("resoudre_adressage", { saisi });
    sortie.replaceChildren(...rendreResolution(r));
  } catch (err) {
    // Erreur de commande (saisie refusée, config absente) : le backend rend
    // déjà un texte en français, on ne le réécrit pas.
    sortie.replaceChildren(h("div", { class: "banner err" }, `❌ ${err}`));
  } finally {
    // Rouvert selon le champ, pas inconditionnellement : vidé pendant que la
    // requête était en vol, il redeviendrait actif au-dessus d'une saisie vide.
    go.disabled = !champ.value.trim();
  }
}

/** Définitions des champs, RECOPIÉES de `docs/legende_champs.md` — une seule
 *  source pour le CSV, le PDF de légende et cet écran. Toute modification là-bas
 *  doit être reportée ici : `client/tests/legende_parite.test.js` échoue sinon. */
const LEGENDE = {
  in_peppol: "existe — L'adressage est-il provisionné dans le réseau Peppol (le SMP répond pour cet identifiant).",
  pa_code: "code PA — Code du point d'accès (Access Point) qui dessert l'adressage.",
  pa_name: "nom PA — Nom du point d'accès.",
  pa_country: "pays PA — Code pays du point d'accès.",
  ubl_extended: "CTC-FR — L'adressage déclare-t-il le support de l'extension française France Invoice UBL Extension (CTC-FR).",
  ctc_activation: "activation CTC — Date d'activation déclarée du support CTC (chaîne SMP brute, ISO 8601).",
  ctc_expiration: "expiration CTC — Date d'expiration déclarée du support.",
  ctc_status: "état CTC — État du support calculé à l'instant de l'export à partir des dates ci-dessus.",
  in_directory: "annuaire Peppol — L'adressage 0225 figure-t-il dans l'annuaire Peppol chargé.",
  annuaire_ppf: "annuaire PPF — Adressage présent dans l'annuaire PPF chargé (au moins une ligne).",
  ppf_active: "PPF actif — Au moins une ligne à un motif de présence actif (ensemble configurable dans les réglages, par défaut C / P).",
  pdp_definie: "PDP définie — Au moins une ligne avec une PDP réelle (pdp_fictive = 0).",
  ppf_usable: "PPF utilisable — Au moins une même ligne à un motif actif configuré (défaut C / P) ET PDP réelle (pdp_fictive = 0).",
};

/** Libellé humain d'un champ (2e ligne d'une vedette, sous le nom technique) :
 *  la première moitié de sa légende. DÉRIVÉ, jamais recopié — une table à part
 *  se périmerait en silence le jour où un libellé change dans
 *  `docs/legende_champs.md`, que seul `LEGENDE` est tenu de suivre. */
function libelle(nom) { return LEGENDE[nom].split(" — ")[0]; }

/** Pourquoi une source se tait. Jamais « false » : « je ne sais pas » et « non »
 *  sont deux réponses différentes. */
const MUETTE = {
  annuaire_non_charge: "annuaire jamais chargé",
  annuaire_vide: "annuaire vide",
  hors_perimetre_0225: "hors périmètre des annuaires (0225)",
};

/** Classe de couleur d'un verdict. Quatre issues, pas deux : `later` basculera
 *  seul le jour de l'activation, le peindre en rouge dirait « disqualifié ». */
function classeVerdict(nom, valeur) {
  if (valeur === null || valeur === undefined || valeur === "") return "verdict-nul";
  if (nom === "ctc_status") {
    return { ready: "verdict-ok", later: "verdict-later", expired: "verdict-ko" }[valeur]
      || "verdict-nul";
  }
  return valeur === true ? "verdict-ok" : "verdict-ko";
}

/** Une ligne de champ. `vedette` agrandit et colore ; sinon rendu discret. */
function ligneChamp(nom, valeur, vedette = false) {
  const texte = valeur === null || valeur === undefined || valeur === "" ? "—" : String(valeur);
  const cle = vedette
    ? h("td", { class: "k" }, nom, h("span", { class: "lib" }, libelle(nom)))
    : h("td", { class: "k" }, nom);
  const val = vedette
    ? h("td", { class: "v" }, h("span", { class: classeVerdict(nom, valeur) }, texte))
    : h("td", { class: `v ${valeur === true ? "t" : valeur === false ? "f" : ""}` }, texte);
  return h("tr", { title: LEGENDE[nom], class: vedette ? "cle" : "" }, cle, val);
}

/** Une source muette : la raison, jamais une valeur. */
function ligneMuette(nom, raison, vedette = false) {
  const cle = vedette
    ? h("td", { class: "k" }, nom, h("span", { class: "lib" }, libelle(nom)))
    : h("td", { class: "k" }, nom);
  return h("tr", { title: LEGENDE[nom], class: vedette ? "cle" : "" },
    cle,
    h("td", { class: "v" }, h("span", { class: "verdict-nul" }, MUETTE[raison] || raison)));
}

/** Une section, avec une `note` facultative du résolveur SOUS l'en-tête et non
 *  dedans : `.resolve-sect-h` est en capitales espacées, où une note technique
 *  devient illisible — et ces notes portent une URL, seule chaîne dont perdre
 *  des caractères fait perdre la réponse à « pourquoi ce verdict ». */
function section(titre, note, ...lignes) {
  return h("div", { class: "resolve-sect" },
    h("div", { class: "resolve-sect-h" }, titre),
    ...(note ? [h("p", { class: "resolve-note" }, note)] : []),
    h("table", {}, ...lignes));
}

/** Rendu complet. Les trois sections sont toujours présentes : une source
 *  muette se dit, elle ne disparaît pas. */
function rendreResolution(r) {
  const out = [];
  out.push(h("p", { class: "resolve-canon" },
    "Résolu comme ", h("span", { class: "v" }, r.canonique), ` · mode ${r.mode}`));

  if (r.reseau.etat === "repond") {
    const c = r.reseau.champs;
    // La latence reste dans l'en-tête : une note ne la remplace pas, les deux
    // se lisent ensemble (un 403 en 2 s ne se diagnostique pas comme en 90 ms).
    out.push(section(`Réseau Peppol · ${r.reseau.latence_ms} ms`, c.note,
      ligneChamp("in_peppol", c.in_peppol),
      ligneChamp("pa_code", c.pa_code),
      ligneChamp("pa_name", c.pa_name),
      ligneChamp("pa_country", c.pa_country),
      ligneChamp("ubl_extended", c.ubl_extended),
      ligneChamp("ctc_activation", c.ctc_activation),
      ligneChamp("ctc_expiration", c.ctc_expiration),
      ligneChamp("ctc_status", c.ctc_status, true)));
  } else {
    out.push(h("div", { class: "banner err" }, `❌ ${r.reseau.message}`));
  }

  out.push(r.annuaire_peppol.etat === "repond"
    ? section("Annuaire Peppol", null, ligneChamp("in_directory", r.annuaire_peppol.in_directory))
    : section("Annuaire Peppol", null, ligneMuette("in_directory", r.annuaire_peppol.raison)));

  out.push(r.ppf.etat === "repond"
    ? section("Annuaire PPF", null,
      ligneChamp("annuaire_ppf", r.ppf.annuaire_ppf),
      ligneChamp("ppf_active", r.ppf.ppf_active),
      ligneChamp("pdp_definie", r.ppf.pdp_definie),
      ligneChamp("ppf_usable", r.ppf.ppf_usable, true))
    : section("Annuaire PPF", null,
      ligneMuette("annuaire_ppf", r.ppf.raison),
      ligneMuette("ppf_usable", r.ppf.raison, true)));

  return out;
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
  // Nom du jeu de paramètres chargé dans la session. Point de départ, pas état
  // à persister : le plan généré, lui, est déjà en base.
  jeu: null,
  filtres: { mep: "", run: "", pa: "", origine: "", etat: "", q: "" },
  genere: false,
  // Rapport au fichier ouvert (clé de `MESSAGES_FICHIER`) : décide du style du
  // bouton de rapprochement dans la barre d'outils du récap. `null` tant
  // qu'aucun plan n'est en mémoire — le bouton reste alors masqué (`genere`
  // gate déjà son affichage).
  rapportFichier: null,
  // Rampe manuelle : { [run_num]: volume }. Vit ici et non dans le DOM — les
  // champs sont dynamiques (un par run retenu) et le panneau est reconstruit
  // en entier à chaque rendu. Un run absent vaut 0 côté moteur, en silence :
  // c'est pourquoi le rendu liste TOUS les runs retenus.
  volumes: {},
};

/** Runs retenus de l'aperçu, en ordre chronologique, avec leur détail chiffré.
 *  Unique source des volumes : la timeline porte déjà l'ordre et les écarts. */
function runsRetenus() {
  const t = plan.apercu?.timeline ?? [];
  return t.flatMap((j) => j.runs.filter((r) => !r.ecart).map((r) => ({ ...r, date: j.date })));
}

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

function planBanner(kind, texte, ...actionNodes) {
  const el = $("plan-banner");
  if (!kind) { el.className = "hidden"; el.replaceChildren(); return; }
  el.className = kind;
  el.replaceChildren(texte, ...actionNodes);
}

function fmtN(n) { return (n ?? 0).toLocaleString("fr-FR"); }

/** Fait parler un bouton pendant un traitement, et le rétablit QUOI QU'IL
 *  ARRIVE — sans le `finally`, une génération qui échoue laisse un bouton mort
 *  et interdit de réessayer.
 *
 *  Deux effets pour un geste : l'indication est là où le regard vient de
 *  cliquer, et le bouton désactivé fait garde de ré-entrance. Aucune de ces
 *  commandes n'émet d'avancement — une barre de progression y serait
 *  décorative, contrairement à celles de l'annuaire et du PPF. */
async function occupe(bouton, pendant, travail) {
  // La garde ne lit PAS `disabled` : ce drapeau sert déjà de validation de
  // formulaire (le bouton « Retirer » reste inerte tant que le motif est vide),
  // et les deux sens se confondraient. `dataset.occupe` ne dit qu'une chose.
  if (!bouton || bouton.dataset.occupe) return;
  const repos = bouton.textContent;
  const inerte = bouton.disabled;
  bouton.dataset.occupe = "1";
  bouton.disabled = true;
  bouton.textContent = pendant;
  try {
    return await travail();
  } finally {
    delete bouton.dataset.occupe;
    // On rend l'état de départ, pas « actif » : un bouton soumis à validation
    // ne doit pas s'ouvrir parce qu'une action a échoué.
    bouton.disabled = inerte;
    bouton.textContent = repos;
  }
}

/** Marque les chiffres de l'écran comme périmés le temps d'un recalcul.
 *
 *  Le problème n'est pas qu'une tâche tourne, c'est que ce qui est affiché ne
 *  correspond plus à ce qu'on vient de saisir : l'atténuation le dit, une barre
 *  de progression laisserait croire l'inverse. Le panneau latéral en est
 *  exclu — c'est la frappe qui déclenche le recalcul, griser ce qu'on règle
 *  serait absurde. */
function marquerRecalcul(actif) {
  $("plan-main").classList.toggle("recalcul", actif);
  $("plan-attente").classList.toggle("hidden", !actif);
}

/** Annonce les fichiers d'une génération précédente retirés du répertoire de
 *  livraison. Le backend fait le ménage — un fichier de MEP périmé peut être
 *  transmis par erreur — mais ce qu'on efface d'un répertoire de livraison ne
 *  s'efface pas en silence. Seul le nom est montré : le chemin complet
 *  noierait le message, et le répertoire est celui qu'on vient d'écrire. */
function signalerObsoletes(obsoletes) {
  const noms = (obsoletes ?? []).map((c) => c.split(/[/\\]/).pop());
  if (!noms.length) return;
  planBanner("info",
    `${noms.length} fichier(s) d'une génération précédente supprimé(s) : ${noms.join(", ")}`);
}

// Bornes reprises de `plan.rs::ANNEES_PLAUSIBLES` — à garder alignées.
const ANNEE_MIN = 2000, ANNEE_MAX = 2100;

/** Vrai tant qu'une date est en train d'être tapée. Un champ date n'a pas de
 *  valeur tant qu'il est incomplet — sauf l'année, qui vaut dès son premier
 *  chiffre : taper « 2026 » notifie l'an 2, l'an 20, l'an 202, puis 2026, et
 *  chacun est une date entière aux yeux du navigateur. Agir dessus déclenchait
 *  un calcul par chiffre, sur une fenêtre partant de l'an 2 que la timeline
 *  parcourt un jour civil à la fois.
 *
 *  Un champ VIDE n'est pas une frappe en cours : c'est un effacement, et il
 *  doit continuer de retirer les chiffres de l'écran. */
function saisieEnCours(v) {
  if (!v) return false;
  const an = Number(v.slice(0, 4));
  return an < ANNEE_MIN || an > ANNEE_MAX;
}

/** Décisions manuelles que porte le plan enregistré, réparties comme
 *  `Preserves::depuis` (plan.rs) les répartit : une ligne retirée l'emporte sur
 *  gelée, gelée sur épinglée. Les compter autrement donnerait un total
 *  supérieur au nombre de lignes — et ferait mentir la fenêtre juste avant un
 *  geste destructeur. */
function decisionsDuPlan() {
  const d = { gelees: 0, epinglees: 0, retirees: 0 };
  for (const l of plan.lignes) {
    if (l.retire_motif != null) d.retirees += 1;
    else if (l.gelee) d.gelees += 1;
    else if (l.origine === "manuel") d.epinglees += 1;
  }
  d.total = d.gelees + d.epinglees + d.retirees;
  return d;
}

/** Ce qu'une remise à zéro emporte, énuméré. Seuls les ensembles non vides
 *  paraissent : « 0 compte retiré » ferait hésiter sur rien. */
function listeDecisions(d) {
  const s = (n) => (n > 1 ? "s" : "");
  const items = [];
  if (d.gelees)
    items.push(h("li", {}, h("b", {}, fmtN(d.gelees)),
      ` MEP gelée${s(d.gelees)} — déjà passée${s(d.gelees)}`));
  if (d.epinglees)
    items.push(h("li", {}, h("b", {}, fmtN(d.epinglees)),
      ` compte${s(d.epinglees)} épinglé${s(d.epinglees)} — ajouté${s(d.epinglees)} ou déplacé${s(d.epinglees)} à la main`));
  if (d.retirees)
    items.push(h("li", {}, h("b", {}, fmtN(d.retirees)),
      ` compte${s(d.retirees)} retiré${s(d.retirees)} — avec leur motif`));
  return h("ul", { class: "perdu" }, ...items);
}

/** Efface les lignes du plan, paramètres du panneau intacts. */
async function effacerLePlan() {
  await invoke("plan_reset");
  plan.genere = false;
  plan.rapportFichier = null;
  await rechargerRecap();
}

async function enregistrerJeu() {
  const f = await save({ filters: [{ name: "YAML", extensions: ["yaml", "yml"] }] });
  if (!f) return;
  try {
    await invoke("plan_params_save", { path: f, params: planParams() });
    plan.jeu = f.split(/[\\/]/).pop();
    planBanner(null);
    renderPlanAside();
  } catch (e) { planBanner("error", `Enregistrement impossible : ${e}`); }
}

async function chargerJeu() {
  const f = await open({ multiple: false, filters: [{ name: "YAML", extensions: ["yaml", "yml"] }] });
  if (!f) return;
  let params;
  try {
    params = await invoke("plan_params_load", { path: f });
  } catch (e) {
    // Refus sec, comme un profil incompatible : AUCUN état modifié. Un jeu à
    // moitié appliqué produirait un plan que personne ne saurait expliquer.
    planBanner("error", `Jeu de paramètres illisible : ${e}`);
    return;
  }
  const nom = f.split(/[\\/]/).pop();
  const d = decisionsDuPlan();
  // Un plan sans décision n'a rien à perdre : on ne demande rien.
  if (!d.total) { poserJeu(params, nom); return; }
  confirmerChargement(params, nom, d);
}

function poserJeu(params, nom) {
  plan.jeu = nom;
  appliquerParams(params);
  planBanner(null);
  planRecalc();
}

function confirmerChargement(params, nom, d) {
  const poser = (effacer) => async (ev) => {
    if (effacer) {
      const fait = await occupe(ev?.currentTarget, "Effacement…", () => effacerLePlan())
        .then(() => true, (e) => { planBanner("error", String(e)); return false; });
      if (!fait) { closeModal(); return; }
    }
    poserJeu(params, nom);
    closeModal();
  };
  modal(
    h("h3", {}, `Charger « ${nom} » ?`),
    h("p", { class: "field-hint" }, "Le plan enregistré porte des décisions manuelles :"),
    listeDecisions(d),
    h("p", { class: "field-hint" },
      "Les conserver les reporte sur le nouveau plan, et sort leurs comptes du tirage. "
      + "Repartir de zéro les efface définitivement."),
    h("div", { class: "actions" },
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"),
      h("button", { onclick: poser(false) }, "Conserver ces décisions"),
      h("button", { class: "btn-danger", onclick: poser(true) }, "Repartir de zéro")));
}

function ouvrirRepartirDeZero() {
  const d = decisionsDuPlan();
  const noeuds = [h("h3", {}, "Repartir de zéro ?")];
  // Un motif de retrait ne se reconstitue pas : c'est la seule perte qu'on ne
  // peut pas refaire à la main, elle se dit à part.
  if (d.retirees)
    noeuds.push(h("div", { class: "danger-note" },
      `Les ${fmtN(d.retirees)} compte(s) retiré(s) perdent leur motif. Cette trace ne se reconstitue pas.`));
  noeuds.push(
    listeDecisions(d),
    h("p", { class: "field-hint" }, "Les paramètres du panneau, eux, sont conservés."),
    h("div", { class: "actions" },
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"),
      h("button", { class: "btn-danger", onclick: (ev) =>
        occupe(ev?.currentTarget, "Effacement…", async () => {
          try { await effacerLePlan(); planRecalc(); }
          catch (e) { planBanner("error", String(e)); }
          closeModal();
        }) }, "Repartir de zéro")));
  modal(...noeuds);
}

/** Paramètres envoyés au moteur. Forme exacte de PlanParams (plan.rs). */
function planParams() {
  const forme = $("plan-forme")?.value ?? "plate";
  // Le pilote n'a AUCUN effet en forme manuelle (`construire_rampe` retourne
  // avant lui) : l'envoyer quand même ferait porter à l'utilisateur un réglage
  // qui n'agit pas. Il est masqué à l'écran, mais la case garde son état —
  // c'est ici que la décision se prend.
  const pilote = forme !== "manuelle" && $("plan-pilote")?.checked
    ? { runs: +$("plan-pilote-runs").value || 0, cf_par_run: +$("plan-pilote-cf").value || 0 }
    : null;
  const rampe = { forme, pilote };
  if (forme === "geometrique") rampe.raison = +$("plan-raison").value || 2;
  // Les volumes partent tels qu'ils sont tenus, SANS passer par l'aperçu : au
  // tout premier recalcul — ouverture d'un plan enregistré — il n'existe pas
  // encore, et les filtrer dessus enverrait une map vide. Le moteur donne 0 à
  // tout run absent de la map, et ignore les clés qui ne désignent plus un run
  // retenu : transmettre l'état brut est exact dans les deux sens.
  if (forme === "manuelle") rampe.volumes = { ...plan.volumes };
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

/** Bloc de saisie des volumes, un champ par run retenu.
 *
 *  Ne se re-rend PAS à la frappe : `oninput` écrit dans `plan.volumes` et
 *  déclenche le recalcul, mais reconstruire le panneau ferait perdre le focus
 *  au champ en cours de saisie. L'alerte de dépassement arrive donc au rendu
 *  suivant, avec l'aperçu — elle vient du moteur, pas d'un calcul local. */
function volumesParRun() {
  const runs = runsRetenus();
  if (!runs.length) {
    return h("p", { class: "field-hint" },
      "Aucun Run de Facturation retenu : rien à répartir pour l'instant.");
  }
  const lignes = runs.map((r) => {
    const champ = h("input", {
      type: "number", min: "0", id: `plan-vol-${r.num}`,
      value: String(plan.volumes[r.num] ?? 0),
      oninput: (e) => {
        plan.volumes[r.num] = Math.max(0, +e.target.value | 0);
        planRecalc();
      },
    });
    // `reliquat` dit ce qu'un calcul « volume > stock » raterait : il tient
    // compte du report entrant. Ce n'est pas une erreur — le surplus part sur
    // le run suivant — d'où l'ambre et non le rouge.
    const reste = r.detail?.reliquat ?? 0;
    const ligne = h("div", { class: reste > 0 ? "vol over" : "vol" },
      h("span", { class: "who" },
        h("b", {}, r.num), ` · ${jourMois(r.date)}`),
      champ);
    if (reste > 0) {
      ligne.append(h("span", { class: "flag" },
        `stock ${fmtN(r.detail.stock)} · ${fmtN(reste)} reportés`));
    }
    return ligne;
  });

  const total = runs.reduce((n, r) => n + (plan.volumes[r.num] ?? 0), 0);
  const atteignables = (plan.apercu?.stock_jj ?? [])
    .reduce((n, s) => n + (s.couvert ? s.comptes : 0), 0);

  return h("div", { id: "plan-volumes" },
    h("div", { class: "vols" }, ...lignes),
    h("div", { class: "vols-foot" },
      h("span", { class: "k" }, "Total saisi"), h("b", {}, fmtN(total))),
    h("div", { class: "vols-foot last" },
      h("span", { class: "k" }, "Pool atteignable"), h("span", { class: "k" }, fmtN(atteignables))),
    h("button", { class: "lnk", id: "plan-vol-zero", onclick: () => {
      plan.volumes = {};
      renderPlanAside();
      planRecalc();
    } }, "Tout à 0"));
}

/** « 2026-08-11 » → « 11/08 ». Les dates de la timeline sont ISO. */
function jourMois(iso) { return `${iso.slice(8, 10)}/${iso.slice(5, 7)}`; }

/** Changement de forme de rampe.
 *
 *  La première bascule vers « manuelle » part des volumes que l'écran affiche
 *  déjà : le geste réel est « je prends ma rampe linéaire et j'ajuste deux
 *  runs », pas « je ressaisis six valeurs ». Une saisie existante n'est jamais
 *  réécrasée — repasser par une forme calculée puis revenir la retrouve. */
function basculerForme(forme) {
  if (forme === "manuelle" && !Object.keys(plan.volumes).length) {
    for (const r of runsRetenus()) plan.volumes[r.num] = r.detail?.vise ?? 0;
  }
  renderPlanAside();
  planRecalc();
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
      await memoriserColonnes();
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

  // Le panneau est reconstruit en entier à chaque rendu, et ces champs-là
  // n'ont d'état QUE dans le DOM — `planParams` les relit sur les éléments.
  // Sans cette capture, le moindre re-rendu (ajouter une MEP, changer une
  // colonne, la forme de rampe) les rendrait à leur valeur par défaut.
  const saisi = (id, defaut) => $(id)?.value ?? defaut;
  const vals = {
    debut: saisi("plan-debut", ""),
    fin: saisi("plan-fin", ""),
    mepcount: saisi("plan-mepcount", "0"),
    cible: saisi("plan-cible", ""),
    raison: saisi("plan-raison", "1.55"),
    piloteRuns: saisi("plan-pilote-runs", "0"),
    piloteCf: saisi("plan-pilote-cf", "0"),
    seed: saisi("plan-seed", "42"),
  };
  // Un booléen se pose après coup : `setAttribute("checked", false)` cocherait
  // la case (l'attribut compte par sa présence, pas par sa valeur).
  const casePilote = h("input", { type: "checkbox", id: "plan-pilote",
    onchange: () => { renderPlanAside(); planRecalc(); } });
  casePilote.checked = pilOn;

  const blocVolumes = forme === "manuelle" ? volumesParRun() : null;

  $("plan-aside").replaceChildren(
    // En tête du panneau : un jeu porte TOUT ce qui suit, calendrier compris.
    h("h3", {}, "Jeu de paramètres"),
    h("div", { class: "duo-btn" },
      h("button", { onclick: enregistrerJeu }, "Enregistrer…"),
      h("button", { onclick: chargerJeu }, "Charger…")),
    plan.jeu
      ? h("p", { class: "field-hint jeu-actif" }, `✓ Chargé depuis « ${plan.jeu} »`)
      : h("p", { class: "field-hint" },
          "Fenêtre, MEP, cible, rampe et calendrier des runs, dans un seul fichier."),

    h("h3", { id: "plan-cols-title", class: manque ? "need" : "" }, "Colonnes"), bloc,

    h("h3", {}, "Calendrier de facturation"),
    h("button", { onclick: importerRuns }, "Charger runs.csv…"),
    h("p", { class: "field-hint" }, plan.runs.length
      ? `✓ ${plan.runs.length} Runs de Facturation chargés.`
      : "En-tête attendu DATE_RUN;NUM_RUN;JJS — date en JJ/MM/AAAA, jours séparés par des tirets."),

    h("h3", {}, "Fenêtre FUT"),
    h("div", { class: "row" },
      h("label", {}, "Début", h("input", { type: "date", id: "plan-debut", value: vals.debut, oninput: planRecalc })),
      h("label", {}, "Fin", h("input", { type: "date", id: "plan-fin", value: vals.fin, oninput: planRecalc }))),

    h("h3", {}, "Mises en production"),
    chips,
    h("label", {}, "Ajouter une MEP"),
    h("input", { type: "date", id: "plan-mepadd", onchange: (e) => {
      const v = e.target.value;
      // Le geste de fin — vider le champ et reconstruire le panneau — détruit
      // le champ en cours de saisie : il n'appartient qu'à la date achevée.
      // Sans cette garde, taper une MEP créait quatre MEP et vidait le champ
      // dès le premier chiffre de l'année.
      if (saisieEnCours(v)) return;
      if (v && !plan.meps.includes(v)) { plan.meps.push(v); plan.meps.sort(); }
      e.target.value = ""; renderPlanAside(); planRecalc();
    } }),
    h("label", {}, "Nombre total visé"),
    h("input", { type: "number", id: "plan-mepcount", min: "0", value: vals.mepcount, style: "width:80px", oninput: planRecalc }),

    h("h3", {}, "Cible"),
    h("label", {}, "Comptes distincts à traiter"),
    h("input", { type: "number", id: "plan-cible", min: "1", placeholder: "auto", value: vals.cible, style: "width:120px", oninput: planRecalc }),
    h("p", { class: "field-hint" }, "Vide = tout le pool éligible atteignable."),
    // La cible n'est PAS neutralisée en manuel : `construire_rampe` l'ignore,
    // mais `allouer` s'en sert encore pour les quotas par plateforme. La griser
    // serait donc faux — d'où cette note.
    ...(forme === "manuelle"
      ? [h("p", { class: "field-hint warn-hint", id: "plan-cible-manuel" },
          "En rampe manuelle, la cible ne fixe plus le volume — les volumes saisis font foi. Elle sert encore de base aux quotas par plateforme.")]
      : []),

    h("h3", {}, "Rampe de montée en charge"),
    h("label", {}, "Forme"),
    h("select", { id: "plan-forme", onchange: () => basculerForme($("plan-forme").value) },
      ...[["plate", "Plate (équirépartie)"], ["lineaire", "Linéaire (croissance douce)"],
          ["geometrique", "Géométrique (raison réglable)"],
          ["manuelle", "Manuelle (volume par run)"]].map(([v, t]) => {
        const o = h("option", { value: v }, t);
        if (v === forme) o.selected = true;
        return o;
      })),
    ...(forme === "geometrique"
      ? [h("label", {}, "Raison"), h("input", { type: "number", id: "plan-raison", min: "1.1", step: "0.05", value: vals.raison, style: "width:90px", oninput: planRecalc })]
      : []),
    ...(blocVolumes ? [h("label", {}, "Volumes par run"), blocVolumes] : []),
    // Le pilote ne s'applique pas en manuel : le laisser à l'écran donnerait un
    // réglage sans effet. `planParams` force `null` de son côté.
    ...(forme === "manuelle" ? [] : [
      h("label", {}, casePilote, " Pilote prudent au démarrage"),
      ...(pilOn
        ? [h("label", {}, "Durée du pilote (runs)"), h("input", { type: "number", id: "plan-pilote-runs", min: "0", value: vals.piloteRuns, style: "width:80px", oninput: planRecalc }),
           h("label", {}, "Comptes par run de pilote"), h("input", { type: "number", id: "plan-pilote-cf", min: "0", value: vals.piloteCf, style: "width:80px", oninput: planRecalc }),
           h("p", { class: "field-hint" }, "Le niveau du pilote sert de socle : la rampe ne redescend jamais en dessous.")]
        : []),
    ]),

    h("h3", {}, "Options"),
    h("label", {}, "Seed"),
    h("input", { type: "number", id: "plan-seed", value: vals.seed, style: "width:100px", oninput: planRecalc }),
    h("p", { class: "field-hint" }, "Départage déterministe à priorité égale. Même seed, même plan."),

    h("button", { id: "btn-plan-gen", class: "btn-primary", style: "width:100%;margin-top:16px", onclick: genererPlan },
      plan.genere ? "Régénérer le plan" : "Générer le plan"),
    h("p", { class: "field-hint" }, "Conservés à l'identique : MEP gelées, retouches manuelles et comptes retirés."),
    // Un bouton destructeur qui ne détruit rien est du bruit : il n'apparaît
    // que quand il y a un plan à jeter.
    ...(plan.lignes.length
      ? [h("button", { class: "btn-danger", style: "width:100%", onclick: ouvrirRepartirDeZero },
          "Repartir de zéro…")]
      : []),
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

const TL_MOIS = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet",
  "août", "septembre", "octobre", "novembre", "décembre"];

// Map plutôt qu'objet littéral : `TL_ECARTS[r.ecart]` traverserait la chaîne
// de prototypes sur une valeur comme "constructor" ou "toString" et rendrait
// un libellé qui n'existe pas — un `Map` n'a pas ce piège.
const TL_ECARTS = new Map([
  ["exclu", "exclu à la main"],
  ["hors_fenetre", "hors fenêtre"],
  ["mep_non_passee", "la première MEP n'est pas encore passée"],
  ["aucune_mep", "aucune MEP n'est définie"],
]);

/** « Juillet 2026 » depuis une date ISO, sans passer par Date (fuseaux). */
function libelleMois(iso) {
  const m = TL_MOIS[+iso.slice(5, 7) - 1];
  return `${m[0].toUpperCase()}${m.slice(1)} ${iso.slice(0, 4)}`;
}

// Le jour porte le marquage week-end/férié — pas la ligne : un run ou une
// MEP tombant ce jour-là reste pleinement lisible, tout en gardant
// l'information visible (pas seulement dans une bulle au survol), y
// compris sur les jours sans aucune ligne dédiée à ce chômage.
function celluleJour(j) {
  const texte = `${j.jour_semaine} ${j.date.slice(8)}`;
  const note = j.ferie ? `férié — ${j.ferie}` : j.weekend ? "week-end" : "";
  if (!note) return h("td", { class: "tl-jour" }, texte);
  return h("td", { class: "tl-jour tl-off" }, texte, h("span", { class: "tl-note" }, ` · ${note}`));
}

function ligneJalon(j, jl) {
  const texte = jl.sorte === "mep" ? `MEP ${jl.rang}`
    : jl.sorte === "debut_fenetre" ? "Début de la fenêtre FUT"
    : jl.sorte === "fin_fenetre" ? "Fin de la fenêtre FUT"
    : `Jalon inconnu (${jl.sorte})`;
  const tr = h("tr", { class: jl.sorte === "mep" ? "tl-mep" : "tl-borne" },
    celluleJour(j),
    h("td", { colspan: "9" }, h("span", { class: "flag" }, texte)));
  return tr;
}

function ligneVide(j) {
  return h("tr", {}, celluleJour(j), h("td", { colspan: "9" }, ""));
}

function ligneRun(j, r) {
  const cb = h("input", { type: "checkbox", onchange: (e) => {
    const cible = plan.runs.find((x) => x.num === r.num);
    if (cible) cible.exclu = e.target.checked;
    planRecalc();
  } });
  // Source de vérité = l'état LOCAL (`plan.runs`), pas l'écho serveur `r.exclu` :
  // un `plan_preview` en vol qui répond après un clic plus récent écraserait
  // sinon la case avec un état déjà périmé. Même principe que les cases
  // plateformes, qui lisent `plan.paExclues`.
  cb.checked = plan.runs.find((x) => x.num === r.num)?.exclu ?? r.exclu;
  const boite = h("td", {}, h("label", { class: "tl-chk" }, cb, " exclure"));

  if (r.ecart) {
    // Un run écarté ne porte pas l'action : on ne peut rien y placer. La
    // cellule reste, vide, pour ne pas décaler la colonne des autres lignes.
    return h("tr", { class: "tl-ecarte" },
      celluleJour(j),
      h("td", {}, r.num),
      h("td", { class: "jj" }, r.jjs.join(" · ")),
      h("td", { colspan: "5", class: "tl-why" }, `écarté — ${TL_ECARTS.get(r.ecart) ?? r.ecart}`),
      boite,
      h("td", { class: "tl-add" }));
  }
  const d = r.detail;
  // Le run est le point d'entrée de l'ajout : l'action vit sur sa ligne. Le
  // jour porteur l'accompagne — `RunJour` n'a pas de date, elle vient de lui.
  // L'action n'a de sens qu'avec un plan ENREGISTRÉ : `plan_ajouter` retouche
  // le plan persisté et refuse sinon. `plan.genere` est le seul miroir fidèle
  // de ce que le backend exige (`charger_plan()` rend Some) — `plan.lignes`
  // est encore l'ancien lot quand `genererPlan` redessine la timeline, et un
  // plan sans ligne le laisserait vide sans qu'il soit absent. Sans plan, la
  // cellule reste vide : un bouton grisé sur chaque ligne serait du bruit
  // permanent, alors qu'on arrive normalement ici sans plan.
  const ajout = h("td", { class: "tl-add" },
    ...(plan.genere
      ? [h("button", { class: "tl-add-btn", onclick: (ev) =>
          // `plan_candidats_run` part AVANT que la fenêtre n'apparaisse : sans
          // cela, le clic reste sans effet visible le temps du scan.
          occupe(ev.currentTarget, "…", () => ouvrirAjoutRun(r, j)) }, "+ Ajouter"),
         // Alléger est la décision inverse, prise au même endroit et sous la
         // même condition : elle porte sur CE run, pas sur une sélection du
         // récap. Sa modale lit `plan.lignes`, déjà en mémoire — rien ne part
         // au backend avant que l'utilisateur n'ait choisi son geste.
         // Même habillage (`tl-add-btn`), classe propre en plus : les deux
         // gestes se ressemblent à l'œil mais ne se confondent pas pour qui
         // les cherche dans le DOM.
         h("button", { class: "tl-add-btn tl-alleger-btn", onclick: (ev) =>
           occupe(ev.currentTarget, "…", () => ouvrirAllegerRun(r, j)) }, "Alléger…")]
      : []));
  return h("tr", { class: "tl-run" },
    celluleJour(j),
    h("td", {}, r.num),
    h("td", { class: "jj" }, r.jjs.join(" · ")),
    h("td", { class: "n" }, fmtN(d.vise)),
    h("td", { class: d.report_entrant ? "n carry" : "n zero" },
      d.report_entrant ? `+${fmtN(d.report_entrant)}` : "—"),
    h("td", { class: "n" }, fmtN(d.stock)),
    h("td", { class: "n" }, fmtN(d.place)),
    h("td", { class: d.reliquat ? "n carry" : "n zero" },
      d.reliquat ? `+${fmtN(d.reliquat)}` : "0"),
    boite,
    ajout);
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
  try {
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
    const retenus = a.timeline.reduce(
      (n, j) => n + j.runs.filter((r) => !r.ecart).length, 0);
    const totalRuns = a.timeline.reduce((n, j) => n + j.runs.length, 0);
    noeuds.push(h("p", { class: "field-hint" },
      `${fmtN(retenus)} run(s) retenu(s) sur ${fmtN(totalRuns)} affiché(s) · rattachement à la dernière MEP passée, celle du jour même comprise. Cocher « exclure » retire le run du plan.`));

    const tbl = h("table", { class: "plan-tl" },
      h("tr", {}, ...[["Jour", ""], ["Run", ""], ["Jours facturés", ""],
                      ["Visé", "n"], ["Report", "n"], ["Stock", "n"],
                      ["Placé", "n"], ["Reliquat", "n"], ["", ""], ["", ""]]
        .map(([t, c]) => h("th", { class: c }, t))));

    let moisCourant = "";
    let premierMois = true;
    for (const j of a.timeline) {
      const mois = j.date.slice(0, 7);
      if (mois !== moisCourant) {
        moisCourant = mois;
        tbl.append(h("tr", { class: premierMois ? "tl-mois tl-mois-1er" : "tl-mois" },
          h("td", { colspan: "10" }, libelleMois(j.date))));
        premierMois = false;
      }
      // Les bornes encadrent le contenu du jour, elles ne le précèdent pas
      // toutes les deux : un run posé pile sur `fin` est RETENU, donc afficher
      // « fin de fenêtre » au-dessus de lui le montrerait hors fenêtre alors
      // qu'il compte. Symétriquement, « début de fenêtre » doit rester au-dessus.
      // Une MEP reste au-dessus aussi : elle passe le matin, les runs du jour
      // tournent après elle et lui sont rattachés — l'ordre d'affichage suit
      // l'ordre de la journée.
      for (const jl of j.jalons.filter((x) => x.sorte !== "fin_fenetre"))
        tbl.append(ligneJalon(j, jl));
      // Une ligne vide ne sert qu'à porter la date d'un jour sans aucune autre
      // ligne : un jalon porte déjà celluleJour(j), lui en ajouter une deuxième
      // dupliquerait la date sans rien montrer de plus.
      if (j.runs.length) for (const r of j.runs) tbl.append(ligneRun(j, r));
      else if (!j.jalons.length) tbl.append(ligneVide(j));
      for (const jl of j.jalons.filter((x) => x.sorte === "fin_fenetre"))
        tbl.append(ligneJalon(j, jl));
    }
    noeuds.push(h("div", { class: "tl-scroll" }, tbl));

    // Aperçu des volumes : une barre par run retenu. Les chiffres sont ceux du
    // moteur (`detail.vise`), jamais recalculés ici — et le graphe disparaît
    // sans run retenu plutôt que de diviser par zéro.
    const retenusDetail = runsRetenus();
    if (retenusDetail.length) {
      const maxVise = Math.max(1, ...retenusDetail.map((r) => r.detail?.vise ?? 0));
      noeuds.push(h("h2", {}, "Volumes par run"));
      noeuds.push(h("p", { class: "field-hint" },
        "Comptes visés par Run de Facturation. En ambre, les runs dont le stock n'a pas absorbé le volume : le surplus part en report sur le run suivant."));
      const vb = h("div", { class: "vol-bars", id: "plan-vol-bars" });
      for (const r of retenusDetail) {
        const vise = r.detail?.vise ?? 0;
        const reste = r.detail?.reliquat ?? 0;
        const titre = reste > 0
          ? `${r.num} du ${jourMois(r.date)} — ${fmtN(vise)} visés, ${fmtN(reste)} reportés`
          : `${r.num} du ${jourMois(r.date)} — ${fmtN(vise)} visés`;
        vb.append(h("div", { class: reste > 0 ? "vol-bar over" : "vol-bar", title: titre },
          h("i", { style: `height:${((vise / maxVise) * 100).toFixed(1)}%` }),
          h("span", {}, fmtN(vise))));
      }
      noeuds.push(vb);
    }

    const totalPool = a.stock_jj.reduce((n, s) => n + s.comptes, 0);
    const atteignables = a.stock_jj.reduce((n, s) => n + (s.couvert ? s.comptes : 0), 0);
    const maxJJ = Math.max(1, ...a.stock_jj.map((s) => s.comptes));
    noeuds.push(h("h2", {}, "Stock par jour de cycle"));
    // Sans aucune MEP, `runs_utilisables` ne retient rien et les 31 jours
    // ressortent non couverts : 31 barres rouges diraient « tout est perdu »
    // alors que le message utile est qu'aucun run n'est encore utilisable.
    if (!a.timeline.some((j) => j.runs.some((r) => !r.ecart))) {
      noeuds.push(h("p", { class: "field-hint" },
        "Aucun Run de Facturation n'est retenu : la couverture des jours de cycle ne veut encore rien dire."));
    }
    noeuds.push(h("p", { class: "field-hint" },
      "Comptes du pool éligible, par jour de cycle de facturation. En rouge, les jours qu'aucun run retenu ne couvre : ces comptes sont hors d'atteinte tant que le calendrier ou la fenêtre ne change pas."));
    const barres = h("div", { class: "jj-bars" });
    for (const s of a.stock_jj) {
      const titre = s.couvert
        ? `Jour de cycle ${s.jj} — ${fmtN(s.comptes)} comptes — couvert`
        : `Jour de cycle ${s.jj} — ${fmtN(s.comptes)} comptes — aucun run retenu ne le couvre`;
      barres.append(h("div", { class: s.couvert ? "jj-bar" : "jj-bar no", title: titre },
        h("i", { style: `height:${((s.comptes / maxJJ) * 100).toFixed(1)}%` }),
        h("span", {}, String(s.jj))));
    }
    noeuds.push(barres);
    noeuds.push(h("p", { class: "jj-legend" },
      h("b", {}, fmtN(totalPool)), " comptes éligibles · ",
      h("b", {}, fmtN(atteignables)), " atteignables par les runs retenus · ",
      h("b", {}, fmtN(totalPool - atteignables)), " hors d'atteinte."));

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
        h("button", { onclick: (ev) =>
          occupe(ev.currentTarget, "Rapport en cours…", async () => {
            try {
              const p = await invoke("plan_rapport");
              window.__TAURI__.opener?.revealItemInDir(p);
            } catch (e) { planBanner("error", String(e)); }
          }),
        }, "Rapport du plan…")));
      noeuds.push(r);
    }
    box.replaceChildren(...noeuds);
  } catch (e) {
    planBanner("error", `Affichage du plan de charge impossible : ${e}`);
  }
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

/** Champ de recherche du récapitulatif, remplacé à chaque rendu — mémorisé pour
 *  savoir s'il avait le focus avant de l'être. */
let champRecherche = null;

function renderPlanRecap() {
  const box = $("plan-recap");
  // La recherche déclenche elle-même le rendu qui détruit son champ. Le panneau
  // latéral règle le cas en ne se reconstruisant pas pendant une saisie
  // (`suivreApercuDansLePanneau`) ; ici c'est impossible, le rendu EST le
  // filtrage. On rend donc le focus au champ reconstruit : sans lui, la lettre
  // suivante tombe dans le vide et la recherche repart de zéro à chaque frappe.
  const cherchait = champRecherche !== null && document.activeElement === champRecherche;
  // Rendre le focus ne suffit pas : le champ neuf porte sa valeur par ATTRIBUT,
  // qui ne déplace pas le curseur. Il restait donc à zéro et chaque frappe
  // s'insérait devant la précédente — taper « abc » écrivait « cba ». On relève
  // la position AVANT de reconstruire, tant que le champ est encore l'ancien.
  const curseur = cherchait ? [champRecherche.selectionStart, champRecherche.selectionEnd] : null;
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
    champRecherche = h("input", { class: "grow", type: "search", value: plan.filtres.q,
      placeholder: "Rechercher un compte, un adressage, une raison sociale…",
      oninput: (e) => { plan.filtres.q = e.target.value; renderPlanRecap(); } }));
  // Point d'entrée du rapprochement : toujours là dès qu'un plan existe, sauf
  // fichier illisible (rien à rapprocher). Discret si le fichier ouvert est
  // déjà celui qui a produit le plan, mis en avant sinon — l'éligibilité
  // dépend aussi de l'annuaire et des résolutions, qui bougent sans le fichier.
  if (plan.genere) {
    const info = MESSAGES_FICHIER[plan.rapportFichier] ?? MESSAGES_FICHIER.inconnu;
    if (info.entree !== "masque")
      barre.append(h("button", { class: info.entree === "avant" ? "btn-primary" : "",
        onclick: (ev) => ouvrirRapprocher(ev.currentTarget) }, "Rapprocher…"));
  }

  const visibles = lignesFiltrees();
  const actives = plan.lignes.filter((l) => l.retire_motif == null).length;
  const noeuds = [barre];

  if (plan.sel.size) {
    // La réactivation n'apparaît qu'avec une ligne retirée sous la main, et son
    // libellé compte CE SUR QUOI elle agira — sur une sélection mixte, elle ne
    // touche que les retirées plutôt que de refuser la sélection.
    const retirees = selectionRetiree().length;
    noeuds.push(h("div", { class: "plan-selbar" },
      h("span", {}, h("b", {}, String(plan.sel.size)), " compte(s) sélectionné(s)"),
      h("span", { class: "spacer" }),
      h("button", { onclick: ouvrirDeplacer }, "Déplacer vers un run…"),
      h("button", { class: "btn-danger", onclick: ouvrirRetrait }, "Retirer…"),
      ...(retirees
        ? [h("button", { onclick: ouvrirReactivation }, `Réactiver ${retirees} retiré(s)…`)]
        : []),
      h("button", { class: "btn-ghost", onclick: () => { plan.sel.clear(); renderPlanRecap(); } }, "Tout désélectionner")));
  }
  // Pas de bouton d'ajout ici : la décision part du run, donc l'action vit sur
  // la ligne du run dans la timeline (onglet Paramétrage).
  noeuds.push(h("div", { class: "plan-toolbar" },
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
  if (cherchait) {
    champRecherche.focus();
    champRecherche.setSelectionRange(curseur[0], curseur[1]);
  }
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
      h("button", { class: "btn-primary", onclick: (ev) =>
        occupe(ev.currentTarget, "Déplacement en cours…", async () => {
          try {
            const obsoletes = await invoke("plan_deplacer", { cfs, runNum: sel.value });
            plan.sel.clear(); signalerObsoletes(obsoletes); await rechargerRecap();
          } catch (e) { planBanner("error", String(e)); }
          // Après le rechargement, pas avant : fermer d'abord laisserait
          // l'écran figé sans rien pour l'expliquer.
          closeModal();
        }),
      }, "Déplacer")));
}

/** L'avertissement des MEP gelées, partagé par les gestes qui touchent un
 *  fichier DÉJÀ TRANSMIS. Ce qui change d'un geste à l'autre est le moment où
 *  le fichier bougera : un retrait en sort au prochain tirage, une
 *  réactivation y rentre au prochain enregistrement — d'où `quand`, plutôt
 *  qu'une phrase unique qui serait fausse pour l'un des deux.
 *
 *  `null` quand rien n'est gelé : un avertissement qui paraît toujours
 *  n'avertit plus de rien. */
function noteMepGelee(geles, quand) {
  if (!geles.length) return null;
  const dates = [...new Set(geles.map((l) => fmtDateFr(l.mep_date)))].join(", ");
  return h("div", { class: "danger-note" },
    `⚠ ${fmtN(geles.length)} compte(s) appartiennent à une MEP gelée (${dates}). `
    + `Son fichier a déjà été transmis : il changera au prochain ${quand}.`);
}

function ouvrirRetrait() {
  const cfs = [...plan.sel];
  const geles = plan.lignes.filter((l) => cfs.includes(l.cf) && l.gelee);
  const zone = h("textarea", { rows: "3", style: "width:100%",
    placeholder: "Ex. : migration PDP repoussée par le client, compte clôturé, incident connu…" });
  const btn = h("button", { class: "btn-danger", onclick: (ev) =>
    occupe(ev.currentTarget, "Retrait en cours…", async () => {
      try {
        const obsoletes = await invoke("plan_retirer", { cfs, motif: zone.value });
        plan.sel.clear(); signalerObsoletes(obsoletes); await rechargerRecap();
      } catch (e) { planBanner("error", String(e)); }
      closeModal();
    }),
  }, `Retirer ${cfs.length} compte(s)`);
  btn.disabled = true;
  zone.addEventListener("input", () => { btn.disabled = zone.value.trim() === ""; });

  const noeuds = [
    h("h3", {}, `Retirer ${cfs.length} compte(s) du plan`),
    h("p", { class: "field-hint" },
      "Les lignes restent consultables via le filtre « retiré » et ne seront pas replacées par une régénération. Le retrait est annulable."),
  ];
  // Les fichiers sont cumulatifs : retirer d'une MEP livrée change un fichier
  // déjà transmis. C'est assumé, mais ça se dit au moment de l'acte.
  const note = noteMepGelee(geles, "tirage");
  if (note) noeuds.push(note);
  noeuds.push(h("label", { class: "field-hint" }, "Motif du retrait (obligatoire)"), zone,
    h("div", { class: "actions" }, h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"), btn));
  modal(...noeuds);
}

/** Les comptes retirés de la sélection — les seuls que la réactivation touche. */
function selectionRetiree() {
  return plan.lignes.filter((l) => plan.sel.has(l.cf) && l.retire_motif != null);
}

/** Réactive les comptes retirés de la sélection.
 *
 *  Confirmation systématique, même sans MEP gelée : `annuler_retrait` efface le
 *  motif, donc le geste est moins réversible qu'il n'en a l'air — et un seul
 *  chemin vaut mieux que deux. L'avertissement sur les MEP gelées reprend les
 *  termes de `ouvrirRetrait` : les deux gestes changent un fichier déjà
 *  transmis. */
function ouvrirReactivation() {
  const lignes = selectionRetiree();
  const cfs = lignes.map((l) => l.cf);
  const geles = lignes.filter((l) => l.gelee);

  const noeuds = [
    h("h3", {}, `Réactiver ${cfs.length} compte(s)`),
    h("p", { class: "field-hint" },
      "Ils redeviennent livrables et repartiront dans les fichiers de leur MEP. "
      + "Une régénération pourra les replacer sur un autre run."),
  ];
  // TROISIÈME copie de la même phrase, restée en date ISO là où les deux
  // autres disent JJ/MM/AAAA. `noteMepGelee(geles, "enregistrement")` la
  // remplace telle quelle — mais `tests/plan_reactivation.test.js` fige le
  // format ISO, et ce fichier était hors du périmètre de la revue. À basculer
  // en même temps que lui.
  if (geles.length) {
    noeuds.push(h("div", { class: "danger-note" },
      `⚠ ${geles.length} compte(s) appartiennent à une MEP gelée (${[...new Set(geles.map((l) => l.mep_date))].join(", ")}). `
      + "Son fichier a déjà été transmis : il changera au prochain enregistrement."));
  }
  noeuds.push(
    h("p", { class: "field-hint" }, "⚠ Le motif du retrait sera perdu."),
    h("div", { class: "actions" },
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"),
      h("button", { class: "btn-primary", onclick: (ev) =>
        occupe(ev.currentTarget, "Réactivation en cours…", async () => {
          try {
            const obsoletes = await invoke("plan_annuler_retrait", { cfs });
            plan.sel.clear(); signalerObsoletes(obsoletes); await rechargerRecap();
          } catch (e) { planBanner("error", String(e)); }
          closeModal();
        }),
      }, `Réactiver ${cfs.length} compte(s)`)));
  modal(...noeuds);
}

// --- Rapprochement avec le fichier ouvert -------------------------------------
// Le calcul (`rapprochement.rs`) est pur et déjà testé côté Rust : ce qui suit
// n'est que l'affichage de son résultat, groupé par nature comme la maquette
// du 28/07/2026, et l'application en bloc (jamais ligne à ligne).

/** Nombre de lignes affichées par groupe avant de compter le reste — jamais de
 *  troncature muette, comme le plafond du récap (500 lignes), mais un groupe
 *  n'a besoin que de quelques exemples pour se comprendre. */
const RAPPRO_PLAFOND_GROUPE = 5;

/** Raison sociale d'un compte, retrouvée dans le récap déjà chargé : l'écart
 *  (`Ecart` de rapprochement.rs) ne garde que le strict nécessaire à l'action
 *  (cf, nature, action, gelée), mais son `cf` désigne toujours une ligne du
 *  plan courant — les deux sont calculés dans la même requête côté backend. */
function ligneDuPlan(cf) { return plan.lignes.find((l) => l.cf === cf); }

/** Répartit les écarts dans les 5 groupes de la maquette. Une même nature peut
 *  donner deux actions différentes (jour changé : déplacé ou signalé selon la
 *  gelée et l'existence d'un run cible) — c'est l'action, pas seulement la
 *  nature, qui décide du groupe pour ce cas-là. */
function grouperEcarts(ecarts) {
  const g = { eligibilite: [], disparus: [], deplaces: [], signales: [], plateforme: [] };
  for (const e of ecarts) {
    if (e.nature.type === "eligibilite_perdue") g.eligibilite.push(e);
    else if (e.nature.type === "disparu_du_fichier") g.disparus.push(e);
    else if (e.nature.type === "jour_change") (e.action.type === "deplacer" ? g.deplaces : g.signales).push(e);
    else if (e.nature.type === "plateforme_changee") g.plateforme.push(e);
  }
  return g;
}

/** Carte d'un groupe : pastille de couleur, décompte, tableau plafonné.
 *  `null` si le groupe est vide — la revue ne montre jamais de carte vide. */
function carteGroupeEcarts(couleur, titre, ecarts, construireLigne, colonnes) {
  if (!ecarts.length) return null;
  const tbl = h("table", {});
  for (const e of ecarts.slice(0, RAPPRO_PLAFOND_GROUPE)) tbl.append(construireLigne(e));
  if (ecarts.length > RAPPRO_PLAFOND_GROUPE)
    tbl.append(h("tr", {}, h("td", { colspan: String(colonnes), class: "rappro-more" },
      `… ${fmtN(ecarts.length - RAPPRO_PLAFOND_GROUPE)} autre(s)`)));
  return h("div", { class: "rappro-grp" },
    h("div", { class: "rappro-grp-h" },
      h("span", { class: "rappro-dot", style: `background:${couleur}` }),
      h("span", { class: "rappro-n" }, fmtN(ecarts.length)),
      h("span", { class: "rappro-t" }, titre)),
    tbl);
}

function ligneEligibilitePerdue(e) {
  const l = ligneDuPlan(e.cf);
  return h("tr", {},
    h("td", { class: "rappro-cf" }, e.cf),
    h("td", { class: "rappro-rs" }, l?.raison_sociale ?? ""),
    h("td", { class: "rappro-chg" }, `${e.nature.avant} → `,
      h("span", { class: "tag removed" }, e.nature.apres)));
}

function ligneDisparuDuFichier(e) {
  const l = ligneDuPlan(e.cf);
  const nom = (state.inputPath ?? "").split(/[\\/]/).pop();
  return h("tr", {},
    h("td", { class: "rappro-cf" }, e.cf),
    h("td", { class: "rappro-rs" }, l?.raison_sociale ?? ""),
    h("td", { class: "rappro-chg" }, `absent de ${nom}`));
}

function ligneDeplacement(e) {
  const l = ligneDuPlan(e.cf);
  const a = e.action; // { type: "deplacer", run_num, run_date, mep_id, mep_date }
  const memeMep = l && l.mep_id === a.mep_id;
  return h("tr", {},
    h("td", { class: "rappro-cf" }, e.cf),
    h("td", { class: "rappro-rs" }, l?.raison_sociale ?? ""),
    h("td", { class: "rappro-chg" }, `jour ${e.nature.avant} → ${e.nature.apres}`),
    h("td", { class: "rappro-arrow" },
      `${l?.run_num ?? "?"} → ${a.run_num} · `,
      memeMep ? `MEP ${a.mep_id} inchangée` : h("b", {}, `MEP ${l?.mep_id ?? "?"} → ${a.mep_id}`)));
}

/** Les trois raisons de signalement (jour illisible, MEP gelée, aucun run
 *  cible) se lisent sur la même ligne plutôt que trois groupes d'une poignée
 *  d'écarts : l'action est la même dans les trois cas — aucune. `apres === 0`
 *  est la sentinelle de `rapprochement.rs` pour un jour illisible : hors du
 *  domaine 1–31, elle ne doit JAMAIS s'afficher comme un chiffre. */
function ligneSignalee(e) {
  const l = ligneDuPlan(e.cf);
  const { avant, apres } = e.nature;
  let chg, motif;
  if (apres === 0) {
    chg = h("span", { class: "tag stale" }, "jour de cycle illisible");
    motif = "valeur illisible dans le fichier";
  } else if (e.gelee) {
    chg = h("span", {}, `jour ${avant} → ${apres} `, h("span", { class: "tag frozen" }, "❄ gelé"));
    motif = "un lot livré ne se déplace pas";
  } else {
    chg = `jour ${avant} → ${apres}`;
    motif = `aucun run retenu ne couvre le jour ${apres}`;
  }
  return h("tr", {},
    h("td", { class: "rappro-cf" }, e.cf),
    h("td", { class: "rappro-rs" }, l?.raison_sociale ?? ""),
    h("td", { class: "rappro-chg" }, chg),
    h("td", { class: "rappro-arrow" }, motif));
}

function lignePlateformeChangee(e) {
  const l = ligneDuPlan(e.cf);
  return h("tr", {},
    h("td", { class: "rappro-cf" }, e.cf),
    h("td", { class: "rappro-rs" }, l?.raison_sociale ?? ""),
    h("td", { class: "rappro-chg" }, `${e.nature.avant} → ${e.nature.apres}`));
}

/** Un seul bloc pour les avertissements DÉRIVÉS DU CALCUL (`rapprochement.
 *  avertissements`) : ils décrivent tous ce que le rapprochement va FAIRE
 *  (ampleur, répartition par plateforme), le backend ne les distingue pas
 *  entre eux et les reconnaître au vol serait fragile au moindre changement
 *  de formulation côté Rust. L'avertissement d'annuaire, lui, est SÉPARÉ
 *  (`annuaire_incomplet`) : il ne dérive pas du calcul mais de l'état de la
 *  base, et dit autre chose — que le calcul est incomplet, pas ce qu'il va
 *  faire. Voir `blocAnnuaireIncomplet`. */
function blocAvertissementsRapprochement(liste) {
  if (!liste.length) return null;
  return h("div", { class: "rappro-avert" },
    h("b", {}, "À savoir avant d'appliquer :"),
    h("ul", {}, ...liste.map((a) => h("li", {}, a))));
}

/** Bloc de gravité supérieure (fond rouge, comme la maquette) : l'annuaire PPF
 *  est cumulatif, une éligibilité PPF perdue peut n'y être pas détectable —
 *  un « 0 » peut vouloir dire « je ne sais pas les voir ». Affiché en tête,
 *  avant les avertissements du calcul : il qualifie leur fiabilité même. */
function blocAnnuaireIncomplet(texte) {
  if (!texte) return null;
  return h("div", { class: "rappro-avert rappro-avert-hard" }, h("b", {}, texte));
}

/** Texte du récapitulatif de bas de revue. Toutes les lignes que le calcul a
 *  vues (hors retirées, déjà hors plan) sont soit inchangées soit en écart :
 *  additionner les deux donne le total actif AVANT application. */
function rapproRecapTexte(rapprochement) {
  const retraits = rapprochement.ecarts.filter((e) => e.action.type === "retirer").length;
  const avant = rapprochement.inchangees + rapprochement.ecarts.length;
  return `Après application : ${fmtN(avant - retraits)} ligne(s) active(s), `
    + `${fmtN(retraits)} retirée(s). Le reste du plan n'est pas retiré au sort.`;
}

/** Bandeau de compte rendu après application. Les comptages viennent du calcul
 *  déjà affiché à l'écran (retirés/déplacés/plateformes) ; `obsoletes` et
 *  `rapport` sont ce que remonte `plan_rapprocher_appliquer` — le nombre de
 *  fichiers de MEP réécrits n'est pas remonté par le backend, et ne se devine
 *  pas : on ne l'affiche donc pas.
 *
 *  Le rapport, lui, se nomme : une fois la modale fermée, le bandeau est la
 *  seule trace qu'un document est parti avec les fichiers. Réduit à son nom —
 *  le chemin de la machine qui a produit le lot n'apprend rien. */
function compteRenduRapprochement(rapprochement, obsoletes, rapport, retraitsManuels = 0) {
  const g = grouperEcarts(rapprochement.ecarts);
  const parts = [];
  const retraits = g.eligibilite.length + g.disparus.length;
  if (retraits) parts.push(`${fmtN(retraits)} compte(s) retiré(s)`);
  if (g.deplaces.length) parts.push(`${fmtN(g.deplaces.length)} déplacé(s)`);
  if (g.plateforme.length) parts.push(`${fmtN(g.plateforme.length)} plateforme(s) corrigée(s)`);
  // Sans aucun changement appliqué, ce clic n'a produit qu'un document : le
  // dire ainsi, plutôt qu'annoncer un « rapprochement appliqué » qui n'a
  // touché à rien.
  const doc = `${fmtN(retraitsManuels)} retrait(s) manuel(s) documenté(s)`;
  let texte = parts.length
    ? `✓ Rapprochement appliqué : ${parts.join(", ")}.` + (retraitsManuels ? ` ${doc}.` : "")
    : retraitsManuels
      ? `✓ Note de livraison produite : ${doc}, aucun compte modifié.`
      : "✓ Rapprochement appliqué.";
  const noms = (obsoletes ?? []).map((c) => c.split(/[/\\]/).pop());
  if (noms.length) texte += ` ${noms.length} fichier(s) obsolète(s) supprimé(s) : ${noms.join(", ")}.`;
  if (rapport) texte += ` Rapport : ${rapport.split(/[/\\]/).pop()}.`;
  planBanner("ok", texte);
}

/** Revue groupée, rien n'est encore appliqué. `empreinte` est celle vue au
 *  calcul : elle voyage jusqu'au clic sur Appliquer sans jamais transiter par
 *  le DOM (dataset, attribut…), qui se reconstruit à chaque re-rendu du récap.
 *  `annuaireIncomplet` est séparé de `rapprochement.avertissements` (voir
 *  `blocAnnuaireIncomplet`) : rendu à part, en tête. */
function renderRevueRapprochement(rapprochement, empreinte, annuaireIncomplet, retraitsManuels = 0) {
  const g = grouperEcarts(rapprochement.ecarts);
  // "Signaler" ne mute rien (`rapprochement::appliquer`) : ce n'est pas un
  // changement à appliquer, même si c'est un écart à lire.
  const changements = rapprochement.ecarts.filter((e) => e.action.type !== "signaler").length;

  const groupes = [
    carteGroupeEcarts("var(--red)", "à retirer — éligibilité perdue",
      g.eligibilite, ligneEligibilitePerdue, 3),
    carteGroupeEcarts("var(--red)", "à retirer — disparus du fichier",
      g.disparus, ligneDisparuDuFichier, 3),
    carteGroupeEcarts("var(--gold)", "à déplacer — jour de cycle changé",
      g.deplaces, ligneDeplacement, 4),
    carteGroupeEcarts("var(--amber)", "signalés — aucune action automatique",
      g.signales, ligneSignalee, 4),
    carteGroupeEcarts("var(--ppf-l3)", "plateforme corrigée — la ligne ne bouge pas",
      g.plateforme, lignePlateformeChangee, 3),
  ].filter(Boolean);
  if (rapprochement.inchangees > 0)
    groupes.push(h("div", { class: "rappro-grp rappro-mute" },
      h("div", { class: "rappro-grp-h" },
        h("span", { class: "rappro-dot", style: "background:var(--green-later)" }),
        h("span", { class: "rappro-n" }, fmtN(rapprochement.inchangees)),
        h("span", { class: "rappro-t" }, "inchangées — non touchées"))));

  const appliquerBtn = h("button", { class: "btn-primary", onclick: (ev) =>
    occupe(ev.currentTarget, "Application en cours…", async () => {
      try {
        const { obsoletes, rapport } = await invoke("plan_rapprocher_appliquer", { empreinte });
        closeModal();
        plan.rapportFichier = "identique"; // le backend vient d'aligner meta.hash dessus
        await rechargerRecap();
        compteRenduRapprochement(rapprochement, obsoletes, rapport, retraitsManuels);
      } catch (e) {
        // Refus (empreinte périmée) ou autre échec : la revue affichée décrit
        // un calcul qui n'est plus valide, la fermer évite de laisser croire
        // qu'elle tient encore. Le bandeau, lui, reste — avec de quoi relancer.
        closeModal();
        planBanner("error", String(e),
          h("button", { class: "btn-primary",
            onclick: (ev2) => ouvrirRapprocher(ev2.currentTarget) }, "Rapprocher…"));
      }
    }) }, `Appliquer ${fmtN(changements)} changement(s)`);
  // N'arrive normalement pas ici (le calcul renvoie plus tôt sans écart), mais
  // si tous les écarts sont de purs signalements, rien n'est à écrire : un
  // bouton inerte vaut mieux qu'un aller-retour serveur qui n'écrirait rien.
  if (!changements) appliquerBtn.disabled = true;

  // L'avertissement d'annuaire d'abord (gravité supérieure : il qualifie la
  // fiabilité du calcul), puis ceux du calcul, puis les groupes.
  const scrollChildren = [
    blocAnnuaireIncomplet(annuaireIncomplet),
    blocAvertissementsRapprochement(rapprochement.avertissements),
    ...groupes,
  ].filter(Boolean);
  modal(
    h("div", { class: "add-head" }, h("h3", {}, "Rapprocher le plan avec le fichier ouvert")),
    h("div", { class: "add-scroll" }, ...scrollChildren),
    h("div", { class: "add-foot" },
      h("span", { class: "rappro-recap" }, rapproRecapTexte(rapprochement)),
      h("span", { class: "spacer" }),
      appliquerBtn,
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler")));
  $("modal").classList.add("modal-wide");
}

/** Point d'entrée : calcule sans rien écrire, puis affiche le résultat — vide
 *  (aucun écart) ou revue groupée par nature. */
async function ouvrirRapprocher(bouton) {
  let vue;
  await occupe(bouton, "Calcul en cours…", async () => {
    try { vue = await invoke("plan_rapprocher"); }
    catch (e) { planBanner("error", String(e)); }
  });
  if (!vue) return;
  const { rapprochement, empreinte, annuaire_incomplet: annuaireIncomplet,
          retraits_manuels: retraitsManuels = 0 } = vue;
  if (!rapprochement.ecarts.length) {
    renderSansEcart(rapprochement, empreinte, retraitsManuels);
    return;
  }
  renderRevueRapprochement(rapprochement, empreinte, annuaireIncomplet, retraitsManuels);
}

/** Aucun écart. Deux situations, un seul écran : soit il n'y a rien à écrire —
 *  déclencheur inerte, comme avant —, soit des retraits faits à la main
 *  attendent d'être documentés, et l'application a un livrable à produire sans
 *  toucher à un seul compte. Le libellé s'adapte à ce sur quoi il agit, comme
 *  « Réactiver n retiré(s)… ».
 *
 *  Le DÉTAIL des retraits n'est pas listé : celui qui applique vient de les
 *  faire, et la revue sert à valider ce qui va être décidé, pas à relire ce qui
 *  l'a déjà été. Le récap et son filtre « retiré » répondent à « lesquels ». */
function renderSansEcart(rapprochement, empreinte, retraitsManuels) {
  const aEcrire = retraitsManuels > 0;
  const bouton = aEcrire
    ? h("button", { class: "btn-primary", onclick: (ev) =>
        occupe(ev.currentTarget, "Production en cours…", async () => {
          try {
            const { obsoletes, rapport } = await invoke("plan_rapprocher_appliquer", { empreinte });
            closeModal();
            plan.rapportFichier = "identique"; // le backend vient d'aligner meta.hash dessus
            await rechargerRecap();
            compteRenduRapprochement(rapprochement, obsoletes, rapport, retraitsManuels);
          } catch (e) {
            // Même traitement que la revue : la modale décrit un calcul qui
            // n'est plus valide, la fermer évite de laisser croire qu'il tient.
            closeModal();
            planBanner("error", String(e),
              h("button", { class: "btn-primary",
                onclick: (ev2) => ouvrirRapprocher(ev2.currentTarget) }, "Rapprocher…"));
          }
        }) }, "Produire la note de livraison")
    : h("button", {}, "Appliquer");
  if (!aEcrire) bouton.disabled = true;

  const corps = [h("p", {}, aEcrire
    ? `✓ Aucun écart avec le fichier ouvert. ${fmtN(rapprochement.inchangees)} ligne(s) active(s).`
    : `✓ Le plan est à jour avec le fichier ouvert. `
      + `${fmtN(rapprochement.inchangees)} ligne(s) active(s), aucun écart.`)];
  if (aEcrire) {
    corps.push(h("div", { class: "rappro-avert" },
      h("b", {}, `${fmtN(retraitsManuels)} retrait(s) fait(s) à la main ne figurent dans aucune note transmise.`),
      " Les comptes concernés ont déjà quitté les fichiers de MEP ; ce qui manque, "
      + "c'est le document qui l'explique au destinataire."));
    corps.push(h("p", { class: "rappro-recap" },
      "Aucun compte ne bouge : la note est écrite, les fichiers de MEP sont réécrits "
      + "à l'identique et le plan se réaligne sur le fichier ouvert."));
  }

  modal(
    h("h3", {}, "Rapprochement du plan"),
    ...corps,
    h("div", { class: "add-foot" },
      h("span", { class: "spacer" }),
      bouton,
      h("button", { class: "btn-ghost", onclick: closeModal }, "Fermer")));
}

/** Tri d'une liste de candidats sur une colonne. Ne mute pas l'entrée. */
function trierCandidats(liste, colonne, croissant) {
  const val = (c) => (colonne === "jj" ? c.jj : String(c[colonne] ?? "").toLowerCase());
  return [...liste].sort((a, b) => {
    const x = val(a), y = val(b);
    const d = x < y ? -1 : x > y ? 1 : 0;
    return croissant ? d : -d;
  });
}

/** Filtres combinés. Un filtre vide ne restreint rien. */
function filtrerCandidats(liste, f) {
  const t = (f.texte ?? "").trim().toLowerCase();
  return liste.filter((c) => {
    if (t && !`${c.cf} ${c.raison_sociale}`.toLowerCase().includes(t)) return false;
    if (f.pa && c.pa !== f.pa) return false;
    if (f.ctc && c.ctc_status !== (f.ctc === "(vide)" ? "" : f.ctc)) return false;
    if (f.ppf === "oui" && !c.ppf_usable) return false;
    if (f.ppf === "non" && c.ppf_usable) return false;
    return true;
  });
}

/** Pastille de statut CTC. La valeur reste BRUTE (`ready`, `later`…), y compris
 *  à l'écran : c'est la même que dans le fichier d'export et les autres sorties,
 *  écart assumé à la règle « texte UI en français ». */
function pastilleCtc(s) {
  const classe = { ready: "st-ready", later: "st-later", expired: "st-expired" }[s] ?? "st-none";
  return h("span", { class: `st ${classe}` }, s || "(vide)");
}

/** Ajout de comptes SUR UN RUN donné (`RunJour` de la timeline, avec son jour
 *  civil porteur). Le run est fixé par l'appel : plus de sélecteur de run, donc
 *  plus d'intersection à calculer — le backend ne rend que les comptes dont le
 *  jour de cycle est couvert par ce run. */
async function ouvrirAjoutRun(run, jour) {
  let candidats;
  try { candidats = await invoke("plan_candidats_run", { runNum: run.num }); }
  catch (e) { return planBanner("error", String(e)); }
  // Deux causes mènent à une liste vide, et on ne sait pas laquelle : ou bien
  // aucun compte du fichier ne porte un jour de cycle couvert par ce run — le
  // cas le plus fréquent — ou bien tous ceux qui en portent un sont déjà au
  // plan. Le message dit les deux plutôt que d'en affirmer une.
  if (!candidats.length)
    return planBanner("info", `Aucun compte à proposer pour le run ${run.num} : aucun compte du fichier ne porte un jour de cycle couvert par ce run, ou tous ceux qui en portent un sont déjà au plan.`);

  const choisis = new Set();
  let tri = { colonne: "cf", croissant: true };
  const filtres = { texte: "", pa: "", ctc: "", ppf: "" };

  const recherche = h("input", { type: "search", placeholder: "Rechercher un compte, une raison sociale…" });
  const selPa = h("select", {}, h("option", { value: "" }, "Toutes les plateformes"),
    ...[...new Set(candidats.map((c) => c.pa))].filter(Boolean).sort()
      .map((p) => h("option", { value: p }, p)));
  const selCtc = h("select", {}, h("option", { value: "" }, "CTC : tous"),
    ...["ready", "later", "expired", "(vide)"].map((s) => h("option", { value: s }, s)));
  const selPpf = h("select", {}, h("option", { value: "" }, "PPF : tous"),
    h("option", { value: "oui" }, "utilisable"), h("option", { value: "non" }, "non utilisable"));
  const raz = h("button", { class: "reset", onclick: () => {
    recherche.value = ""; selPa.value = ""; selCtc.value = ""; selPpf.value = "";
    Object.assign(filtres, { texte: "", pa: "", ctc: "", ppf: "" });
    dessiner();
  } }, "réinitialiser");

  const corps = h("div", { class: "add-scroll" });
  const pied = h("span", { class: "add-count" });

  const enTete = (cle, libelle, classe = "") =>
    h("th", {
      class: `sortable ${classe} ${tri.colonne === cle ? "sorted" : ""}`.trim(),
      onclick: () => {
        tri = { colonne: cle, croissant: tri.colonne === cle ? !tri.croissant : true };
        dessiner();
      },
    }, tri.colonne === cle ? `${libelle} ${tri.croissant ? "▲" : "▼"}` : libelle);

  /** Case à cocher dont l'état vient de `checked`, jamais de l'attribut :
   *  `setAttribute("checked", false)` COCHE la case. */
  const caseACocher = (coche, onchange) => {
    const cb = h("input", { type: "checkbox", onchange });
    cb.checked = coche;
    return cb;
  };

  function dessiner() {
    const vus = trierCandidats(filtrerCandidats(candidats, filtres), tri.colonne, tri.croissant);
    // L'état de la case « tout » suit la sélection : reconstruite décochée, un
    // second clic recocherait ce qui l'est déjà au lieu de tout décocher.
    const toutCoche = caseACocher(vus.length > 0 && vus.every((c) => choisis.has(c.cf)), (ev) => {
      for (const c of vus) ev.target.checked ? choisis.add(c.cf) : choisis.delete(c.cf);
      dessiner();
    });
    corps.replaceChildren(h("table", { class: "plan-data" },
      h("tr", {},
        h("th", { style: "width:1%" }, toutCoche),
        enTete("cf", "Compte"), enTete("raison_sociale", "Raison sociale"),
        enTete("jj", "JJ", "n"), enTete("pa", "Plateforme"),
        enTete("ctc_status", "CTC"), enTete("ppf_usable", "PPF")),
      ...vus.map((c) => h("tr", { class: `${c.eligible ? "" : "warn"} ${choisis.has(c.cf) ? "sel" : ""}`.trim() },
        h("td", {}, caseACocher(choisis.has(c.cf), (ev) => {
          ev.target.checked ? choisis.add(c.cf) : choisis.delete(c.cf);
          dessiner();
        })),
        h("td", { class: "cf" }, c.cf),
        h("td", {}, c.raison_sociale),
        h("td", { class: "n jj" }, String(c.jj)),
        h("td", { class: "pa" }, c.pa),
        h("td", {}, pastilleCtc(c.ctc_status)),
        h("td", {}, h("span", { class: `st ${c.ppf_usable ? "st-yes" : "st-no"}` },
          String(c.ppf_usable)))))));
    const forces = [...choisis].filter((cf) => !candidats.find((c) => c.cf === cf)?.eligible).length;
    pied.replaceChildren(
      h("b", {}, String(choisis.size)), " compte(s) sélectionné(s)",
      ...(forces ? [" · ", h("span", { class: "warn-n" }, `${forces} non pleinement éligible(s)`)] : []),
      h("br", {}),
      // « éligible(s) » serait faux : la liste contient DÉLIBÉRÉMENT les
      // comptes non pleinement éligibles, que le compteur juste au-dessus
      // dénombre. « proposé(s) » est ce que la liste est vraiment.
      h("span", { style: "font-size:12px" },
        `${fmtN(candidats.length)} compte(s) proposé(s) à ce run · ${fmtN(vus.length)} affiché(s) après filtres`));
  }

  for (const [el, cle] of [[recherche, "texte"], [selPa, "pa"], [selCtc, "ctc"], [selPpf, "ppf"]]) {
    el.addEventListener(el.tagName.toLowerCase() === "select" ? "change" : "input", () => {
      filtres[cle] = el.value; dessiner();
    });
  }
  dessiner();

  modal(
    h("div", { class: "add-head" },
      h("h3", { style: "margin:2px 0 0" }, `Ajouter des comptes au run ${run.num}`),
      h("div", { class: "add-run" },
        h("span", {}, "Run ", h("b", {}, run.num), " du ", h("b", {}, fmtDateFr(jour.date))),
        // Pas de mention de MEP : elle viendrait de l'aperçu VIVANT, alors que
        // `plan_candidats_run` et `plan_ajouter` résolvent le run sur le
        // calendrier PERSISTÉ. Changer `mep_count` sans regénérer suffit à les
        // faire diverger, et le bandeau annoncerait une MEP que l'ajout ne
        // suivrait pas. Ce qui reste — numéro de run, date, jours de cycle —
        // est revérifié côté backend au moment de l'ajout.
        h("span", { class: "jjs" }, "jours de cycle couverts ",
          ...run.jjs.map((j) => h("code", {}, String(j))))),
      h("p", { class: "field-hint", style: "margin-top:-4px" },
        "Seuls les comptes dont le jour de cycle est couvert par ce run sont listés — un run "
        + "ne peut pas facturer un autre jour. Les comptes non prêts sont proposés et signalés "
        + "(⚠) : les ajouter reste un choix assumé."),
      h("div", { class: "add-filters" }, recherche, selPa, selCtc, selPpf, raz)),
    corps,
    h("div", { class: "add-foot" }, pied, h("span", { class: "spacer" }),
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"),
      h("button", { class: "btn-primary", onclick: (ev) => {
        if (!choisis.size) return;
        return occupe(ev.currentTarget, "Ajout en cours…", async () => {
          try {
            const obsoletes = await invoke("plan_ajouter", { cfs: [...choisis], runNum: run.num });
            signalerObsoletes(obsoletes); await rechargerRecap();
          } catch (e) { planBanner("error", String(e)); }
          closeModal();
        });
      } }, `Ajouter au run ${run.num}`)));

  // La fenêtre porte un tableau : #modal plafonne à 460px, il lui faut sa
  // variante large. Posée après `modal()`, qui la retire à chaque ouverture.
  $("modal").classList.add("modal-wide");
}

/** Date du jour au format ISO, en heure LOCALE. `toISOString()` rendrait la
 *  date UTC : en soirée d'été, un run du jour passerait pour joué.
 *
 *  Aucun test ne verrouille ce choix : le harnais tourne dans le fuseau de la
 *  machine, et le figer demanderait de piloter l'horloge du realm. Le verrou
 *  est la lecture de cette ligne, puis l'application. */
function aujourdhuiIso() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Epoch en SECONDES → « JJ/MM/AAAA », en heure locale pour la même raison
 *  qu'`aujourdhuiIso`. */
function fmtDateEpochFr(secondes) {
  const d = new Date(secondes * 1000);
  const p = (n) => String(n).padStart(2, "0");
  return `${p(d.getDate())}/${p(d.getMonth() + 1)}/${d.getFullYear()}`;
}

/** Badge d'origine d'une ligne, ou `null` pour une ligne simplement allouée —
 *  ce qu'une ligne épinglée coûte à retirer se lit là. */
function badgeOrigineAlleger(l) {
  if (l?.origine === "manuel") return h("span", { class: "epingle" }, "📌 ajouté à la main");
  if (l?.origine === "couverture") return h("span", { class: "epingle" }, "📌 couverture");
  return null;
}

/** Pourquoi CE compte est proposé au retrait plutôt qu'un autre : les deux
 *  critères de l'ordre de sortie du décimage (`plan.rs`, l'inverse de
 *  `trier_par_priorite`) — hors annuaire d'abord, puis les résolutions les
 *  plus anciennes. Sans cette colonne, la proposition est à prendre ou à
 *  laisser sans qu'on sache sur quoi l'amender. */
function justificationRetrait(l) {
  return l?.in_directory
    ? h("span", { class: "why" }, `résolu le ${fmtDateEpochFr(l.resolved_at)}`)
    : h("span", { class: "st st-none" }, "hors annuaire");
}

/** Alléger un run : retirer plusieurs comptes d'un coup, en UN SEUL retrait —
 *  une date, un motif, une écriture. C'est ce qui permet au rapport de
 *  rapprochement de regrouper les comptes sous un chapeau au lieu de répéter
 *  143 fois la même phrase.
 *
 *  La date du run décide de ce qui est offert : un run déjà joué ne se
 *  rééquilibre pas, il s'exclut en entier ; un run à venir se décime
 *  (répartition des plateformes conservée) ou se réduit à une sélection.
 *  Aucune répartition n'est calculée ici : elle vient de
 *  `plan_proposer_retrait`, l'IHM ne fait qu'afficher et amender. */
async function ouvrirAllegerRun(run, jour) {
  // `plan.lignes` peut décrire le plan D'AVANT : la timeline se redessine
  // pendant une génération, bien avant que le récap ne soit rechargé (cf. le
  // commentaire de `ligneRun`). On relit donc l'état avant de compter, comme
  // `ouvrirAjoutRun` interroge le backend avant d'afficher. L'exclusion, elle,
  // ne s'en contente pas : sa liste de comptes est établie côté moteur.
  try { plan.lignes = await invoke("plan_lignes"); }
  catch (e) { return planBanner("error", String(e)); }

  const actifs = plan.lignes.filter((l) => l.run_num === run.num && l.retire_motif == null);
  if (!actifs.length)
    return planBanner("info", `Aucun compte actif sur le run ${run.num} : rien à alléger.`);

  const passe = jour.date < aujourdhuiIso();
  // Dit QUEL run est exclu, jamais POURQUOI : le bouton reste inerte tant que
  // rien n'est écrit après le tiret, car c'est la cause que le rapport
  // transmettra au destinataire six mois plus tard.
  const PREREMPLI = `Run ${run.num} du ${fmtDateFr(jour.date)} exclu a posteriori — `;

  let mode = passe ? "exclure" : "prorata";
  const gardes = new Set();   // mode « ne garder que » : les comptes cochés
  let proposition = null;     // mode prorata : [{pa, retirer[], actifs}], amendable
  let erreurProposition = null;
  let echangeOuvert = null;   // compte dont le panneau d'échange est déplié

  // Le pré-remplissage passe par la PROPRIÉTÉ : un textarea ignore un attribut
  // `value`, son contenu est son enfant texte.
  const zone = h("textarea", { rows: "3", style: "width:100%;margin:8px 0" });
  zone.value = passe ? PREREMPLI : "";
  // Pas de `max` : le vrai maximum est la somme par plateforme des (effectif
  // − 1), que seul le moteur connaît. En annoncer un plus grand ferait mentir
  // le champ, et sur un run d'un seul compte `max` valait 0 pour un `min` de
  // 1. Le backend rend l'erreur en français, dans la modale.
  const champN = h("input", { type: "number", min: "1" });
  const corps = h("div", { class: "alleger-corps" });
  const avert = h("div", {});
  const compte = h("span", { class: "add-count" });
  const bascule = h("div", { class: "modes" });

  const cfsARetirer = () => {
    if (mode === "exclure") return actifs.map((l) => l.cf);
    if (mode === "selection") return actifs.filter((l) => !gardes.has(l.cf)).map((l) => l.cf);
    return (proposition ?? []).flatMap((g) => g.retirer);
  };
  // En mode exclusion, il ne suffit pas d'écrire : il faut avoir écrit
  // AUTRE CHOSE que ce qui était proposé. Comparer des longueurs refuserait
  // « Run joué sans les comptes », plus court que le pré-remplissage et
  // pourtant une cause. C'est l'intention qui se mesure, pas le volume.
  const motifSuffisant = () => {
    const m = zone.value.trim();
    return mode === "exclure" ? m !== "" && m !== PREREMPLI.trim() : m !== "";
  };

  const btn = h("button", { class: "btn-danger", onclick: (ev) =>
    occupe(ev.currentTarget, "Retrait en cours…", async () => {
      try {
        // Exclure un run, c'est TOUT le run : le moteur établit lui-même la
        // liste au moment du clic, sur le plan enregistré. Envoyer les comptes
        // affichés ferait porter le geste par un instantané de l'IHM.
        const obsoletes = mode === "exclure"
          ? await invoke("plan_exclure_run", { runNum: run.num, motif: zone.value })
          : await invoke("plan_retirer", { cfs: cfsARetirer(), motif: zone.value });
        plan.sel.clear(); signalerObsoletes(obsoletes); await rechargerRecap();
      } catch (e) { planBanner("error", String(e)); }
      closeModal();
    }) }, "");

  /** Ce qui dépend de l'état sans dépendre du mode : l'avertissement de MEP
   *  gelée, le pied compteur et le bouton. Les nœuds sont STABLES — un bouton
   *  reconstruit perdrait l'écouteur qui vient de le rouvrir. */
  function rafraichir() {
    const cfs = cfsARetirer();
    // Les fichiers sont cumulatifs : retirer d'une MEP livrée change un fichier
    // déjà transmis. C'est assumé, mais ça se dit au moment de l'acte.
    const note = noteMepGelee(actifs.filter((l) => cfs.includes(l.cf) && l.gelee), "tirage");
    avert.replaceChildren(...(note ? [note] : []));
    compte.replaceChildren(...(mode === "selection"
      ? [h("b", {}, fmtN(gardes.size)), " gardé(s) — ", h("b", {}, fmtN(cfs.length)), " seront retiré(s)"]
      : mode === "prorata" && cfs.length
        ? [h("b", {}, fmtN(cfs.length)), " seront retiré(s) — ",
           h("b", {}, fmtN(actifs.length - cfs.length)), " resteront actifs sur ce run"]
        : []));
    btn.textContent = `Retirer ${fmtN(cfs.length)} compte(s)`;
    // Vider un run à venir n'est pas un geste de cet écran : ne rien garder
    // reviendrait à l'exclure, ce qui ne s'offre que pour un run déjà joué —
    // et laisserait le plan sans run pour ces comptes sans le dire.
    const videraitLeRun = mode === "selection" && gardes.size === 0;
    btn.disabled = !cfs.length || videraitLeRun || !motifSuffisant();
  }

  const corpsExclure = () => [
    h("div", { class: "mode-uniq" }, h("b", {}, "Exclure le run"),
      ` — les ${fmtN(actifs.length)} lignes actives du run sont retirées du plan, épinglées comprises.`),
    h("p", { class: "field-hint" },
      "Les lignes restent consultables via le filtre « retiré » et ne seront pas replacées "
      + "par une régénération. Le retrait est annulable."),
  ];

  /** Candidats à l'échange : les actifs de LA MÊME plateforme que la
   *  proposition n'a pas retenus. Un échange ne change pas le nombre de
   *  retirés de la plateforme — la répartition, et le plancher d'un compte
   *  actif, tiennent quel que soit le choix. */
  function panneauEchange(g, cf) {
    const candidats = actifs.filter((l) => l.pa === g.pa && !g.retirer.includes(l.cf));
    return h("div", { class: "echange" },
      h("div", { class: "t" }, "Remplacer ", h("b", {}, cf),
        ` par un autre compte ${g.pa} de ce run :`),
      ...(candidats.length
        ? candidats.map((l) => h("div", { class: "cand" },
            h("span", { class: "cf" }, l.cf),
            h("span", { class: "rs" }, l.raison_sociale),
            justificationRetrait(l),
            // `append(null)` insérerait le texte « null » dans un vrai DOM —
            // une chaîne vide est le motif du reste du fichier.
            badgeOrigineAlleger(l) ?? "",
            h("button", { class: "lien", onclick: () => {
              g.retirer[g.retirer.indexOf(cf)] = l.cf;
              echangeOuvert = null;
              dessinerCorps();
            } }, "choisir")))
        : [h("div", { class: "cand" }, h("span", { class: "why" },
            "aucun autre compte actif de cette plateforme sur ce run"))]));
  }

  /** Un groupe de la proposition. Le compte « retirés sur actifs » porté par
   *  l'en-tête est ce qui prouve la répartition conservée. */
  function groupeProposition(g) {
    const part = Math.round((g.actifs / actifs.length) * 100);
    const lignes = [];
    for (const cf of g.retirer) {
      const l = actifs.find((x) => x.cf === cf);
      const ouvert = echangeOuvert === cf;
      lignes.push(h("div", { class: "pa-row" },
        h("span", { class: "cf" }, cf),
        h("span", { class: "rs" }, l?.raison_sociale ?? ""),
        justificationRetrait(l),
        // Une ligne simplement allouée n'a pas de badge, et `append(null)`
        // écrirait « null » dans un vrai DOM. Le faux DOM avale les `null`
        // (dom_shim.js) : ce cas ne peut se voir qu'à l'écran.
        badgeOrigineAlleger(l) ?? "",
        h("button", { class: "lien", onclick: () => {
          echangeOuvert = ouvert ? null : cf;
          dessinerCorps();
        } }, ouvert ? "fermer" : "échanger")));
      if (ouvert) lignes.push(panneauEchange(g, cf));
    }
    return h("div", { class: `pa-grp${g.retirer.length ? "" : " vide"}` },
      h("div", { class: "pa-grp-h" },
        h("span", { class: "pa-n" }, g.pa),
        h("span", {}, `— ${fmtN(g.retirer.length)} sur ${fmtN(g.actifs)}`),
        h("span", { style: "flex:1" }),
        h("span", { class: "pa-part" }, g.retirer.length
          ? `${part} % du run · ${fmtN(g.actifs - g.retirer.length)} resteront actifs`
          : `${part} % du run · aucun compte proposé`)),
      ...lignes);
  }

  const proposer = h("button", { onclick: (ev) =>
    occupe(ev.currentTarget, "Calcul en cours…", async () => {
      const n = Number(champN.value);
      proposition = null; echangeOuvert = null; erreurProposition = null;
      if (!Number.isInteger(n) || n < 1) {
        erreurProposition = "Indique le nombre de comptes à retirer (au moins 1).";
      } else {
        try {
          // La proposition est COPIÉE : l'échange l'amende sur place, et le
          // retour du backend n'est pas à nous.
          proposition = (await invoke("plan_proposer_retrait", { runNum: run.num, n }))
            .map((g) => ({ pa: g.pa, actifs: g.actifs, retirer: [...g.retirer] }));
        } catch (e) { erreurProposition = String(e); }
      }
      // La modale reste ouverte : le champ que le message invite à corriger
      // est dedans, et le bandeau du plan est derrière elle.
      dessinerCorps();
    }) }, "Proposer");

  const corpsProrata = () => [
    h("div", { class: "prop-saisie" },
      h("label", {}, "Retirer"), champN,
      h("span", { class: "field-hint", style: "margin:0" }, `compte(s) sur ${fmtN(actifs.length)} actifs`),
      proposer),
    h("p", { class: "field-hint" },
      "La part de chaque plateforme dans le run est conservée. Une plateforme n'est jamais "
      + "vidée : son dernier compte actif reste."),
    ...(erreurProposition ? [h("div", { class: "danger-note" }, erreurProposition)] : []),
    ...(proposition ? [h("div", { class: "add-scroll" }, ...proposition.map(groupeProposition))] : []),
  ];

  const corpsSelection = () => [
    h("p", { class: "field-hint" },
      "Cochez les comptes à garder. Tous les autres seront retirés du plan. La répartition "
      + "des plateformes n'est pas préservée : c'est votre choix qui décide."),
    h("div", { class: "add-scroll" }, h("table", { class: "plan-data" },
      h("tr", {}, h("th", { style: "width:34px" }, "Garder"), h("th", {}, "N° de CF"),
        h("th", {}, "Raison sociale"), h("th", {}, "Plateforme"), h("th", {}, "Jour"),
        h("th", {}, "Origine")),
      ...actifs.map((l) => {
        // L'état d'une case ne vient QUE de `checked` : `setAttribute("checked",
        // false)` la COCHE.
        const cb = h("input", { type: "checkbox", onchange: (ev) => {
          ev.target.checked ? gardes.add(l.cf) : gardes.delete(l.cf);
          dessinerCorps();
        } });
        cb.checked = gardes.has(l.cf);
        const epingle = badgeOrigineAlleger(l);
        return h("tr", { class: gardes.has(l.cf) ? "sel" : "" },
          h("td", {}, cb),
          h("td", { class: "cf" }, l.cf),
          h("td", {}, l.raison_sociale),
          h("td", { class: "pa" }, l.pa),
          h("td", { class: "jj" }, String(l.jj)),
          // `table.plan-data td.pa` atténue la cellule : la classe se pose sur
          // le `td`, pas sur un span à l'intérieur, sinon « tirage » ressort
          // autant qu'une origine qui, elle, mérite le regard.
          epingle ? h("td", {}, epingle) : h("td", { class: "pa" }, "tirage"));
      }))),
  ];

  function dessinerCorps() {
    corps.replaceChildren(...(mode === "exclure" ? corpsExclure()
      : mode === "prorata" ? corpsProrata() : corpsSelection()));
    rafraichir();
  }

  /** Deux gestes différents, pas deux valeurs d'un même réglage : des segments
   *  lisibles sans être ouverts, plutôt qu'une liste déroulante. */
  function majBascule() {
    if (passe) return;
    bascule.replaceChildren(...[["prorata", "Retirer N — répartition conservée"],
                                ["selection", "Ne garder que ma sélection"]]
      .map(([cle, libelle]) => h("button", { class: mode === cle ? "on" : "", onclick: () => {
        if (mode === cle) return;
        mode = cle;
        majBascule();
        dessinerCorps();
      } }, libelle)));
  }

  zone.addEventListener("input", rafraichir);
  majBascule();
  dessinerCorps();

  modal(
    h("div", { class: "add-head" },
      h("h3", { style: "margin:2px 0 0" }, `Alléger le run ${run.num}`),
      h("div", { class: "add-run" },
        h("span", {}, "Run ", h("b", {}, run.num), " du ", h("b", {}, fmtDateFr(jour.date))),
        // Les jours de cycle disent ce qu'on POURRAIT encore placer sur ce
        // run : sans objet pour un run déjà joué, qu'on ne fait que quitter.
        ...(passe ? [] : [h("span", { class: "jjs" }, "jours de cycle ",
          ...(run.jjs ?? []).map((j) => h("code", {}, String(j))))]),
        h("span", { class: "jjs" }, `${fmtN(actifs.length)} comptes actifs`)),
      bascule),
    corps, avert,
    h("label", { class: "field-hint" }, "Motif du retrait (obligatoire)"), zone,
    h("div", { class: "add-foot" }, compte, h("span", { class: "spacer" }), btn,
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler")));
  // Un run passé n'affiche aucune liste : la modale garde sa largeur de
  // confirmation. Les deux autres modes listent des comptes.
  if (!passe) $("modal").classList.add("modal-wide");
}

// --- Cycle de vie de l'écran -------------------------------------------------
let planRecalcTimer = null;
/** Recalcul débounce : chaque frappe déclencherait sinon un scan complet. */
function planRecalc() {
  clearTimeout(planRecalcTimer);
  planRecalcTimer = setTimeout(async () => {
    const p = planParams();
    // Frappe en cours : ne rien calculer ET ne rien effacer. Les chiffres de la
    // dernière fenêtre complète restent, plutôt que de clignoter à chaque
    // chiffre d'année. Le moteur refuse de son côté ces dates-là.
    if (saisieEnCours(p.debut) || saisieEnCours(p.fin)) return;
    if (!p.runs.length || !p.debut || !p.fin) { plan.apercu = null; renderPlanParam(); return; }
    marquerRecalcul(true);
    try {
      plan.apercu = await invoke("plan_preview", { params: p });
      planBanner(null);
    } catch (e) {
      plan.apercu = null;
      planBanner("warn", String(e));
    } finally {
      // Même en échec : des chiffres grisés à jamais seraient pires que des
      // chiffres faux, on ne saurait plus qu'ils ne bougeront pas.
      marquerRecalcul(false);
    }
    renderPlanParam();
    suivreApercuDansLePanneau();
  }, 250);
}

/** Le panneau latéral porte lui aussi des chiffres venus de l'aperçu — un champ
 *  par run retenu, l'alerte de dépassement — donc il doit suivre les aperçus.
 *  Sans cela, un plan enregistré en rampe manuelle rouvre avec la bonne forme
 *  mais SANS ses champs : le panneau est rendu avant que le premier aperçu
 *  existe, et il fallait changer de forme puis revenir pour les voir.
 *
 *  Sauf pendant une saisie : c'est la frappe qui déclenche le recalcul, et
 *  reconstruire le panneau ferait perdre le focus au champ en cours. */
function suivreApercuDansLePanneau() {
  const actif = document.activeElement;
  if (actif && $("plan-aside").contains(actif)) return;
  renderPlanAside();
}

async function genererPlan() {
  await occupe($("btn-plan-gen"), "Génération en cours…", async () => {
    try {
      const r = await invoke("plan_generate", { params: planParams() });
      plan.apercu = r.apercu;
      plan.fichiers = r.fichiers;
      plan.genere = true;
      // Fraîchement généré depuis le fichier ouvert : par construction, c'est
      // celui-là qui vient de le produire.
      plan.rapportFichier = "identique";
      planBanner(null);
      signalerObsoletes(r.obsoletes);
      renderPlanAside();
      renderPlanParam();
      await rechargerRecap();
    } catch (e) { planBanner("error", String(e)); }
  });
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

/** Le plan enregistré n'est relu qu'UNE fois par session. Fermer l'écran ne
 *  fait que le masquer : tout son état vit encore en mémoire, et le relire à
 *  chaque ouverture écrasait en silence une saisie que l'utilisateur n'avait
 *  pas encore générée. C'est l'arrêt de l'application qui fait repartir de la
 *  dernière génération, pas un aller-retour sur l'écran.
 *
 *  Le drapeau est posé AVANT la lecture : une lecture en échec est annoncée
 *  par son bandeau, et la réessayer à l'ouverture suivante risquerait
 *  d'effacer ce qui aurait été saisi entre-temps. */
let planRelu = false;

/** Pose un jeu de `PlanParams` sur le panneau : état en mémoire puis champs.
 *  Partagé par la restauration du plan enregistré et le chargement d'un jeu de
 *  paramètres — les deux appliquent exactement la même forme. */
function appliquerParams(params) {
  plan.runs = params.runs ?? [];
  plan.meps = params.meps ?? [];
  plan.paExclues = new Set(params.pa_exclues ?? []);
  plan.volumes = { ...(params.rampe?.volumes ?? {}) };
  renderPlanAside();
  if ($("plan-debut")) $("plan-debut").value = params.debut ?? "";
  if ($("plan-fin")) $("plan-fin").value = params.fin ?? "";
  if ($("plan-mepcount")) $("plan-mepcount").value = params.mep_count ?? 0;
  if ($("plan-cible")) $("plan-cible").value = params.cible ?? "";
  if ($("plan-seed")) $("plan-seed").value = params.seed ?? 42;
  // La rampe n'était pas restaurée du tout : un plan enregistré en géométrique
  // rouvrait en « plate », sa raison perdue. Les champs qui en dépendent
  // (raison, volumes, pilote) n'existent qu'une fois la forme posée — d'où le
  // second rendu, qui conserve ce qui vient d'être écrit.
  const ra = params.rampe ?? {};
  if ($("plan-forme")) $("plan-forme").value = ra.forme ?? "plate";
  if ($("plan-pilote")) $("plan-pilote").checked = !!ra.pilote;
  renderPlanAside();
  if (ra.raison && $("plan-raison")) $("plan-raison").value = ra.raison;
  if (ra.pilote) {
    if ($("plan-pilote-runs")) $("plan-pilote-runs").value = ra.pilote.runs ?? 0;
    if ($("plan-pilote-cf")) $("plan-pilote-cf").value = ra.pilote.cf_par_run ?? 0;
  }
}

/** Ce que l'écran dit du rapport entre le plan enregistré et le fichier ouvert.
 *  Clés de `RapportAuFichier` (commands.rs) — à garder alignées.
 *
 *  « Même nom, contenu changé » n'est PAS « ce n'est pas le même fichier » :
 *  c'en est un, mis à jour. Ce qui est en cause n'est pas son identité mais
 *  l'âge de ce que le plan affirme, d'où le libellé. */
// `entree` pilote le bouton de rapprochement de la barre d'outils du récap :
// "discret" (style par défaut) tant que rien n'affirme un écart, "avant"
// (mis en avant) dès que le fichier a bougé, "masque" quand il n'y a rien à
// rapprocher (fichier illisible). L'éligibilité dépend aussi de l'annuaire et
// des résolutions, qui changent sans le fichier : même "identique" garde un
// bouton, discret.
const MESSAGES_FICHIER = {
  identique: { pied: "", bandeau: null, entree: "discret" },
  contenu_different: {
    pied: " — ⚠ son contenu a changé depuis",
    bandeau: (f) => `Le fichier ouvert porte le même nom que celui qui a produit le plan `
      + `(« ${f} ») mais son contenu a changé : les lignes gelées décrivent des comptes `
      + `tels qu'ils étaient, pas tels qu'ils sont.`,
    entree: "avant",
  },
  autre_fichier: {
    pied: " — ⚠ le fichier ouvert est différent",
    bandeau: (f) => `Le plan enregistré a été produit depuis « ${f} », différent du `
      + `fichier ouvert : les lignes gelées peuvent ne plus correspondre.`,
    entree: "avant",
  },
  // Fichier absent ou illisible : on ne conclut pas. Prétendre « fichier
  // différent » serait une affirmation que rien n'étaye. Rien à rapprocher
  // non plus : le bouton disparaît plutôt que d'ouvrir sur un calcul voué à
  // échouer à la première lecture du fichier.
  inconnu: { pied: " — vérification impossible", bandeau: null, entree: "masque" },
};

/** Restaure paramètres et calendrier du plan enregistré. */
async function hydraterPlan() {
  try {
    const enr = await invoke("plan_load");
    if (enr) {
      plan.genere = true;
      appliquerParams(enr.params);
      // Un état que ce frontend ne connaît pas se rabat sur « inconnu », jamais
      // sur le silence : une absence d'avertissement se lit « tout va bien ».
      // Le rapprochement suit le même repli : un état futur non reconnu masque
      // son bouton plutôt que d'affirmer à tort qu'il n'y a rien à faire.
      plan.rapportFichier = MESSAGES_FICHIER[enr.rapport] ? enr.rapport : "inconnu";
      const m = MESSAGES_FICHIER[plan.rapportFichier];
      $("plan-foot-info").textContent = `Plan enregistré depuis ${enr.fichier}${m.pied}`;
      if (m.bandeau) planBanner("warn", m.bandeau(enr.fichier));
    } else {
      renderPlanAside();
    }
  } catch (e) {
    renderPlanAside();
    planBanner("warn", String(e));
  }
}

async function ouvrirPlan() {
  $("plan-screen").classList.remove("hidden");
  $("plan-sub").textContent = state.inputPath
    ? state.inputPath.split(/[\\/]/).pop() : "";
  // Le mapping a pu changer depuis l'entrée dans l'étape Run (profil chargé,
  // étape Format revisitée) : on resynchronise avant tout calcul.
  await pousserConfig();
  if (!planRelu) { planRelu = true; await hydraterPlan(); }
  // L'onglet où on était, pas « Paramètres » d'office : revenir sur l'écran
  // n'est pas le rouvrir.
  planShowTab(plan.tab);
  await rechargerRecap();
  planRecalc();
}

function fermerPlan() { $("plan-screen").classList.add("hidden"); }

$("btn-plan-open").addEventListener("click", ouvrirPlan);
$("btn-plan-close").addEventListener("click", fermerPlan);
$("btn-plan-back").addEventListener("click", fermerPlan);
document.querySelectorAll(".plan-tab").forEach((b) =>
  b.addEventListener("click", () => planShowTab(b.dataset.ptab)));
