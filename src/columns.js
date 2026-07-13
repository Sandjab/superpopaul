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

// Réordonnancement des colonnes par SortableJS (vendor/Sortable.min.js) en
// mode forceFallback : le drag-and-drop HTML5 est avalé par le handler
// drag-drop natif de la webview Tauri (dragDropEnabled=true, requis pour
// déposer un FICHIER sur le dropzone). Le fallback de Sortable n'émet que des
// événements pointeur, insensibles à ce réglage.
let sortable = null;

// Fait suivre les cellules du corps à l'ordre courant des en-têtes : pendant
// le drag, Sortable ne déplace que les th ; on réaligne les td via data-idx
// (l'index de colonne au moment du render).
function syncBodyToHeaders() {
  const order = [...$("out-preview").rows[0].cells].map((th) => +th.dataset.idx);
  for (const tr of [...$("out-preview").rows].slice(1)) {
    const byIdx = new Map([...tr.cells].map((td) => [+td.dataset.idx, td]));
    tr.append(...order.map((i) => byIdx.get(i)));
  }
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
  const attrs = { class: c.source, "data-idx": i };
  if (c.source === "peppol")
    attrs.title = "Champ calculé par l'API Peppol — les valeurs affichées sont un exemple.";
  return h("th", attrs, `⠿ ${colLabel(c)} `, rm);
}

function renderOutPreview() {
  sortable?.destroy();
  sortable = null;
  const cols = state.config.output.columns;
  const rows = state.preview ? state.preview.rows : [];
  const cell = (c, r, i) => {
    if (c.source === "peppol")
      return h("td", { class: "muted", "data-idx": i }, PEPPOL_SAMPLE[c.field]);
    const idx = state.preview.headers.indexOf(c.name);
    return h("td", { "data-idx": i }, idx >= 0 ? (r[idx] ?? "") : "");
  };
  if (cols.length === 0) {
    $("out-preview").replaceChildren(h("tr", {}, h("td", { class: "muted" },
      "Toutes les colonnes sont exclues — utilise « + Ajouter une colonne »")));
  } else {
    const head = h("tr", {}, ...cols.map(makeHeader));
    $("out-preview").replaceChildren(
      head,
      ...rows.map((r) => h("tr", {}, ...cols.map((c, i) => cell(c, r, i)))),
    );
    sortable = new Sortable(head, {
      animation: 250,
      forceFallback: true,          // jamais de DnD HTML5 (cf. commentaire ci-dessus)
      fallbackOnBody: true,
      ghostClass: "drag-ghost",     // placeholder dans le tableau
      fallbackClass: "drag-fallback", // clone qui suit le curseur
      filter: ".rm",                // le ✕ garde son clic, pas de drag depuis lui
      onChange: syncBodyToHeaders,
      onEnd: () => {
        const order = [...$("out-preview").rows[0].cells].map((th) => +th.dataset.idx);
        cols.splice(0, cols.length, ...order.map((i) => cols[i]));
        renderOutPreview();
      },
    });
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
