// Étape 2 : le tableau de sortie AVEC données d'exemple est l'outil de
// configuration : drag latéral des en-têtes, ✕ pour exclure, menu « + » pour
// ajouter champs Peppol ou colonnes exclues.
// Source de vérité : state.config.output.columns.

const PEPPOL_FIELDS = [
  ["exists", "existe"], ["pa_code", "code PA"], ["pa_name", "nom PA"],
  ["pa_country", "pays PA"], ["extended_ctc_fr", "CTC-FR"],
];
const PEPPOL_SAMPLE = { exists: "true", pa_code: "PA0042", pa_name: "ACME PA",
                        pa_country: "FR", extended_ctc_fr: "false" };

// Réordonnancement des colonnes au POINTEUR (pointerdown/move/up), pas en
// drag-and-drop HTML5 : ce dernier est avalé par le handler drag-drop natif de
// la webview Tauri (dragDropEnabled=true, requis pour déposer un FICHIER sur le
// dropzone). Les pointer events, eux, ne dépendent pas de ce réglage.
let dragFrom = null;

function clearDragOver() {
  document.querySelectorAll("#out-preview th.dragover")
    .forEach((el) => el.classList.remove("dragover"));
}

// Index de colonne sous le pointeur (via la cellule survolée), ou null.
function colUnderPointer(e) {
  const cell = document.elementFromPoint(e.clientX, e.clientY)
    ?.closest("#out-preview td, #out-preview th");
  return cell ? cell.cellIndex : null;
}

// En-tête (th) d'une colonne d'index donné, pour le retour visuel .dragover.
function headerAt(idx) {
  const head = $("out-preview").firstElementChild;   // 1re ligne = les en-têtes
  return head ? head.children[idx] : null;
}

function colLabel(c) {
  return c.source === "input" ? c.name
       : "⚡ " + PEPPOL_FIELDS.find(([f]) => f === c.field)[1];
}

function makeHeader(c, i) {
  const rm = h("span", {
    class: "rm", title: "Exclure",
    onclick: (e) => {
      e.stopPropagation();
      state.config.output.columns.splice(i, 1);
      renderOutPreview();
    },
  }, "✕");
  const attrs = { class: c.source };
  if (c.source === "peppol")
    attrs.title = "Champ calculé par l'API Peppol — les valeurs affichées sont un exemple.";
  const th = h("th", attrs, `⠿ ${colLabel(c)} `, rm);
  // Drag au pointeur : on capture le pointeur pour suivre le curseur même hors
  // du th, on surligne la colonne cible, on réordonne au relâchement. Démarrage
  // ignoré sur le ✕ (qui garde son clic) et hors clic principal.
  th.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || e.target.closest(".rm")) return;
    dragFrom = i;
    th.setPointerCapture(e.pointerId);
    th.style.cursor = "grabbing";
  });
  th.addEventListener("pointermove", (e) => {
    if (dragFrom === null) return;
    clearDragOver();
    const to = colUnderPointer(e);
    if (to !== null && to !== dragFrom) headerAt(to)?.classList.add("dragover");
  });
  th.addEventListener("pointerup", (e) => {
    if (dragFrom === null) return;
    const from = dragFrom;
    const to = colUnderPointer(e);
    dragFrom = null;
    th.style.cursor = "";
    clearDragOver();
    if (to === null || to === from) return;
    const cols = state.config.output.columns;
    cols.splice(to, 0, cols.splice(from, 1)[0]);
    renderOutPreview();
  });
  th.addEventListener("pointercancel", () => {
    dragFrom = null;
    th.style.cursor = "";
    clearDragOver();
  });
  return th;
}

function renderOutPreview() {
  const cols = state.config.output.columns;
  const rows = state.preview ? state.preview.rows : [];
  const cell = (c, r) => {
    if (c.source === "peppol") return h("td", { class: "muted" }, PEPPOL_SAMPLE[c.field]);
    const idx = state.preview.headers.indexOf(c.name);
    return h("td", {}, idx >= 0 ? (r[idx] ?? "") : "");
  };
  if (cols.length === 0) {
    $("out-preview").replaceChildren(h("tr", {}, h("td", { class: "muted" },
      "Toutes les colonnes sont exclues — utilise « + Ajouter une colonne »")));
  } else {
    $("out-preview").replaceChildren(
      h("tr", {}, ...cols.map(makeHeader)),
      ...rows.map((r) => h("tr", {}, ...cols.map((c) => cell(c, r)))),
    );
  }
  renderAddColMenu();
}

/** Menu « + » : champs Peppol absents puis colonnes d'entrée exclues. */
function renderAddColMenu() {
  const cols = state.config.output.columns;
  const addBtn = (label, spec, cls) =>
    h("button", { class: cls || "", onclick: () => { cols.push(spec); renderOutPreview(); } }, label);
  const peppol = PEPPOL_FIELDS
    .filter(([f]) => !cols.some((c) => c.source === "peppol" && c.field === f))
    .map(([f, label]) => addBtn(`⚡ ${label}`, { source: "peppol", field: f }));
  const inputs = (state.preview ? state.preview.headers : [])
    .filter((name) => !cols.some((c) => c.source === "input" && c.name === name))
    .map((name) => addBtn(name, { source: "input", name }));
  const all = [...peppol, ...inputs];
  $("add-col-menu").replaceChildren(
    ...(all.length ? all : [h("span", { class: "muted" }, "tout est déjà inclus")]),
  );
}

$("btn-add-col").addEventListener("click", () =>
  $("add-col-menu").classList.toggle("hidden"));
