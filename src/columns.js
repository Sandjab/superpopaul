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

function colLabel(c) {
  return c.source === "input" ? c.name
       : "⚡ " + PEPPOL_FIELDS.find(([f]) => f === c.field)[1];
}

function makeHeader(c, i) {
  const rm = h("span", {
    class: "rm", title: "Exclure", draggable: "false",
    onclick: (e) => {
      e.stopPropagation();
      state.config.output.columns.splice(i, 1);
      renderOutPreview();
    },
  }, "✕");
  const th = h("th", { class: c.source, draggable: "true" }, `⠿ ${colLabel(c)} `, rm);
  // L'index source voyage dans dataTransfer : le drop ignore ainsi les drags
  // étrangers (sélection de texte, fichier) qui ne portent pas d'index.
  th.addEventListener("dragstart", (e) =>
    e.dataTransfer.setData("text/plain", String(i)));
  th.addEventListener("dragover", (e) => { e.preventDefault(); th.classList.add("dragover"); });
  th.addEventListener("dragleave", () => th.classList.remove("dragover"));
  th.addEventListener("dragend", () =>
    document.querySelectorAll(".dragover").forEach((el) => el.classList.remove("dragover")));
  th.addEventListener("drop", (e) => {
    e.preventDefault();
    const from = parseInt(e.dataTransfer.getData("text/plain"), 10);
    if (Number.isNaN(from) || from === i) return;
    const cols = state.config.output.columns;
    cols.splice(i, 0, cols.splice(from, 1)[0]);
    renderOutPreview();
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
