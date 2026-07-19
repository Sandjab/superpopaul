// Étape 2 : le tableau de sortie AVEC données d'exemple est l'outil de
// configuration. Paradigme unique : le glisser-déposer — réordonner les
// colonnes dans le tableau, les écarter vers la drop zone (#col-zone), les
// réintégrer depuis la zone à l'emplacement voulu.
// Source de vérité : state.config.output.columns (la zone est calculée).

const PEPPOL_FIELDS = [
  ["in_peppol", "existe"], ["pa_code", "code PA"], ["pa_name", "nom PA"],
  ["pa_country", "pays PA"], ["ubl_extended", "CTC-FR"],
  ["ctc_activation", "activation CTC"], ["ctc_expiration", "expiration CTC"],
  ["ctc_status", "état CTC"],
  ["in_directory", "annuaire Peppol"],
];
const PEPPOL_SAMPLE = { in_peppol: "true", pa_code: "PA0042", pa_name: "ACME PA",
                        pa_country: "FR", ubl_extended: "true",
                        ctc_activation: "2026-09-01", ctc_expiration: "",
                        ctc_status: "later", in_directory: "true" };

// SortableJS (vendor/Sortable.min.js) en mode forceFallback : le
// drag-and-drop HTML5 est avalé par le handler drag-drop natif de la webview
// Tauri (dragDropEnabled=true, requis pour déposer un FICHIER sur le
// dropzone de l'étape 1). Le fallback n'émet que des événements pointeur,
// insensibles à ce réglage. Deux listes partagent le groupe "columns" : la
// ligne d'en-têtes et la drop zone.
let sortHead = null;
let sortZone = null;

// Clé stable portée par th, td et chips (data-key). Préfixée par la source :
// un CSV peut avoir une colonne littéralement nommée « exists », qui ne doit
// pas entrer en collision avec le champ Peppol du même nom.
const colKey = (c) => `${c.source}:${c.source === "input" ? c.name : c.field}`;

function specFromKey(key) {
  const i = key.indexOf(":");
  const source = key.slice(0, i), val = key.slice(i + 1);
  return source === "input" ? { source, name: val } : { source, field: val };
}

function colLabel(c) {
  if (c.source === "input") return c.name;
  const icon = c.field === "in_directory" ? "📇" : "⚡";
  return icon + " " + PEPPOL_FIELDS.find(([f]) => f === c.field)[1];
}

// Classe CSS visuelle d'une colonne (accent). La source « peppol » reste la
// vérité métier ; seul l'accent visuel diffère pour l'annuaire (vert).
function colClass(c) {
  if (c.source === "input") return isPidSpec(c) ? "input pid" : "input";
  return c.field === "in_directory" ? "dir" : "peppol";
}

const isPidSpec = (c) =>
  c.source === "input" && c.name === state.config.input.pid_column;

function makeHeader(c) {
  const pid = isPidSpec(c);
  const attrs = { class: colClass(c), "data-key": colKey(c) };
  if (c.source === "peppol")
    attrs.title = c.field === "in_directory"
      ? "Présence déclarative dans l'annuaire Peppol chargé — indépendant du provisionning Peppol"
      : "Champ calculé par l'API Peppol — les valeurs affichées sont un exemple.";
  if (pid)
    attrs.title = "Colonne des adressages — obligatoire en sortie, non écartable.";
  const th = h("th", attrs, "⠿ ");
  if (c.source === "input") {
    const k = h("button", {
      class: "key-btn",
      title: pid ? "Colonne des adressages"
                 : "Désigner comme colonne des adressages",
    }, "🔑");
    if (!pid) k.addEventListener("click", () => designatePid(c.name)); // app.js
    else k.disabled = true;
    th.append(k, " ");
  }
  th.append(colLabel(c));
  return th;
}

// Cellule du corps pour la colonne c et la ligne r du preview. `temp` marque
// une colonne matérialisée pendant un drag entrant (fond bleuté).
function makeCell(c, r, temp) {
  const key = colKey(c);
  if (c.source === "peppol")
    return h("td", { class: temp ? "muted temp" : "muted", "data-key": key },
      PEPPOL_SAMPLE[c.field]);
  const idx = state.preview.headers.indexOf(c.name);
  const cls = [temp ? "temp" : "", isPidSpec(c) ? "pid" : ""].join(" ").trim();
  return h("td", { class: cls, "data-key": key },
    idx >= 0 ? (r[idx] ?? "") : "");
}

// Le corps reflète l'ordre courant des en-têtes EN CONTINU pendant le drag :
// une clé entrante (chip survolant la ligne) est matérialisée avec ses
// données, un en-tête sorti fait disparaître ses cellules. Chaque tr garde
// un pool data-key → td (les td détachés y restent, prêts à revenir).
// Branché sur l'événement change des DEUX listes : en inter-listes,
// Sortable l'émet côté liste source.
function syncBodyToHeaders() {
  const rows = state.preview ? state.preview.rows : [];
  // .children et non .cells : une chip (div) en survol n'est pas une cellule.
  const keys = [...$("out-preview").rows[0].children].map((el) => el.dataset.key);
  for (const tr of [...$("out-preview").rows].slice(1)) {
    tr.replaceChildren(...keys.map((k) => {
      let td = tr._pool.get(k);
      if (!td) {
        td = makeCell(specFromKey(k), rows[tr._row] ?? [], true);
        tr._pool.set(k, td);
      }
      return td;
    }));
  }
}

// Au drop : la ligne d'en-têtes EST la vérité — on la relit, on committe,
// puis re-render complet des deux listes.
function commitFromHeaders() {
  const keys = [...$("out-preview").rows[0].children].map((el) => el.dataset.key);
  const cols = state.config.output.columns;
  cols.splice(0, cols.length, ...keys.map(specFromKey));
  renderOutPreview();
}

/** Drop zone : champs Peppol absents puis colonnes d'entrée écartées. */
function renderColZone() {
  const cols = state.config.output.columns;
  const excluded = [
    ...PEPPOL_FIELDS
      .filter(([f]) => !cols.some((c) => c.source === "peppol" && c.field === f))
      .map(([f]) => ({ source: "peppol", field: f })),
    ...(state.preview ? state.preview.headers : [])
      .filter((name) => !cols.some((c) => c.source === "input" && c.name === name))
      .map((name) => ({ source: "input", name })),
  ];
  $("col-zone").replaceChildren(...excluded.map((c) =>
    h("div", { class: `chip ${colClass(c)}`, "data-key": colKey(c) }, `⠿ ${colLabel(c)}`)));
}

function renderOutPreview() {
  sortHead?.destroy();
  sortZone?.destroy();
  sortHead = sortZone = null;
  const cols = state.config.output.columns;
  const rows = state.preview ? state.preview.rows : [];

  const head = h("tr", {}, ...cols.map(makeHeader));
  $("out-preview").replaceChildren(
    head,
    ...rows.map((r, ri) => {
      const tr = h("tr", {}, ...cols.map((c) => makeCell(c, r, false)));
      tr._row = ri;
      tr._pool = new Map([...tr.children].map((td) => [td.dataset.key, td]));
      return tr;
    }),
  );
  renderColZone();

  const common = {
    animation: 250,
    forceFallback: true,            // jamais de DnD HTML5 (cf. commentaire de tête)
    fallbackOnBody: true,
    ghostClass: "drag-ghost",       // placeholder dans la liste survolée
    fallbackClass: "drag-fallback", // clone qui suit le curseur
    revertOnSpill: true,            // lâcher hors des deux listes = annulation (plugin OnSpill du build vendorisé)
    onChange: syncBodyToHeaders,
    onEnd: () => setTimeout(commitFromHeaders, 0), // laisser Sortable clore son cycle
  };
  sortHead = new Sortable(head, {
    ...common,
    filter: ".key-btn",       // le clic 🔑 désigne, il ne drague pas
    preventOnFilter: false,   // laisser le click natif partir
    // Garde : la colonne des adressages est obligatoire en sortie — son
    // en-tête refuse de partir vers la zone d'écartement.
    group: { name: "columns",
             pull: (_to, _from, dragEl) => !dragEl.classList.contains("pid"),
             put: true },
  });
  // sort: false — on drag vers/depuis la zone, jamais dedans : son ordre est
  // canonique au render et un tri manuel serait défait au re-render suivant.
  sortZone = new Sortable($("col-zone"), { ...common, group: "columns", sort: false });
  // Toute manipulation du tableau peut faire diverger l'état du profil
  // courant — app.js recalcule la barre (nom • modifié, grisage de 💾).
  window.updateProfileBar?.();
}

// Raccourci sans drag : double-clic sur un en-tête = écarter la colonne,
// double-clic sur une chip = la réintégrer en dernière position. Délégation
// sur les conteneurs (ils survivent aux re-renders), même source de vérité
// (columns → renderOutPreview) et même garde 🔑 que le pull du drag.
$("out-preview").addEventListener("dblclick", (e) => {
  const th = e.target.closest("th[data-key]");
  if (!th) return;
  const cols = state.config.output.columns;
  const i = cols.findIndex((c) => colKey(c) === th.dataset.key);
  if (i < 0 || isPidSpec(cols[i])) return; // la 🔑 ne s'écarte pas
  cols.splice(i, 1);
  renderOutPreview();
});
$("col-zone").addEventListener("dblclick", (e) => {
  const chip = e.target.closest(".chip[data-key]");
  if (!chip) return;
  state.config.output.columns.push(specFromKey(chip.dataset.key));
  renderOutPreview();
});
