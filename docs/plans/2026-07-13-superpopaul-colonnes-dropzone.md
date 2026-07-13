# Étape 2 en DnD unique avec drop zone — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** Faire du glisser-déposer le paradigme unique de l'étape 2 « Colonnes » de Super Popaul : une drop zone de colonnes écartées remplace le ✕ et le menu « + », avec matérialisation en direct des colonnes pendant le drag (variante B de la spec).

**Architecture :** Deux listes SortableJS partageant un `group` (ligne d'en-têtes de `#out-preview` + drop zone `#col-zone`). `state.config.output.columns` reste la seule vérité ; la zone est calculée au render. Sync « virtuelle » du corps sur l'événement `change` (pool `data-key → td` par ligne), commit au `onEnd` puis re-render. Invariant « ≥ 1 colonne » étendu à `Config::validate` côté Rust.

**Tech Stack :** Tauri 2 (Rust), vanilla JS + SortableJS 1.15.6 vendorisé (déjà en place), vérification frontend par harnais HTML + Playwright MCP (pas d'infra de test JS dans ce projet — convention établie).

**Spec :** `docs/superpowers/specs/2026-07-13-superpopaul-colonnes-dropzone-design.md`

**Fichiers touchés :**
- Modify : `superpopaul/src-tauri/src/config.rs` (validate + test)
- Modify : `superpopaul/src/columns.js` (réécriture)
- Modify : `superpopaul/src/index.html` (étape 2)
- Modify : `superpopaul/src/styles.css` (chips/zone/temp, suppression `.rm`)
- Modify : `superpopaul/src/app.js` (défaut de colonnes)
- Scratchpad (non commité) : harnais de vérification

**Important — commits :** la Task 1 (Rust) se committe seule. Les Tasks 2-3 (frontend) forment UN commit atomique, passé la vérification de la Task 4 : supprimer `#btn-add-col` du HTML sans retirer son listener de columns.js (ou l'inverse) casse l'app entre deux commits.

---

### Task 1 : Rust — `Config::validate` rejette `columns: []`

**Files:**
- Modify : `superpopaul/src-tauri/src/config.rs` (fn `validate`, ligne ~122 ; tests, après `validate_rejette_batch_size_hors_bornes` ligne ~246)

- [ ] **Step 1 : écrire le test qui échoue**

Dans le module de tests de `config.rs`, après `validate_rejette_batch_size_hors_bornes` :

```rust
#[test]
fn validate_rejette_colonnes_vides() {
    // L'UI (drop zone, garde « min 1 colonne ») garantit ≥ 1 colonne ; un
    // YAML columns: [] chargerait vers un tableau sans ligne d'en-têtes —
    // aucune cible de drop, utilisateur coincé.
    let mut cfg = config_exemple();
    cfg.output.columns.clear();
    assert!(cfg.validate().is_err());
}
```

- [ ] **Step 2 : vérifier qu'il échoue**

Run : `cargo test --manifest-path superpopaul/src-tauri/Cargo.toml validate_rejette_colonnes_vides`
Attendu : FAIL (`assertion failed: cfg.validate().is_err()`)

- [ ] **Step 3 : implémentation minimale**

Dans `Config::validate` (config.rs:122-130), ajouter avant le `Ok(())` :

```rust
if self.output.columns.is_empty() {
    return Err("output.columns ne doit pas être vide".into());
}
```

- [ ] **Step 4 : vérifier que tout passe**

Run : `cargo test --manifest-path superpopaul/src-tauri/Cargo.toml`
Attendu : tous les tests PASS (69+).
Run : `cargo clippy --manifest-path superpopaul/src-tauri/Cargo.toml -- -D warnings`
Attendu : clean.

- [ ] **Step 5 : commit**

```bash
git add superpopaul/src-tauri/src/config.rs
git commit -m "feat(superpopaul): Config::validate exige au moins une colonne de sortie"
```

---

### Task 2 : index.html + styles.css — la drop zone remplace le menu « + »

**Files:**
- Modify : `superpopaul/src/index.html:40-47` (section `#step-columns`)
- Modify : `superpopaul/src/styles.css` (suppression `th .rm`, ajout zone/chips/temp)

**PAS de commit à la fin de cette tâche** (voir en-tête du plan).

- [ ] **Step 1 : remplacer la section étape 2 d'index.html**

Remplacer (lignes 40-47) :

```html
    <!-- Étape 2 : colonnes de sortie (aperçu manipulable) -->
    <section id="step-columns" class="panel hidden">
      <h2>Colonnes du fichier de sortie</h2>
      <p class="muted">Glisse les en-têtes pour réordonner, ✕ pour exclure. L'aperçu montre le résultat final.</p>
      <div><button id="btn-add-col" title="Ajouter un champ Peppol ou réintégrer une colonne exclue.">+ Ajouter une colonne ⚡</button>
           <span id="add-col-menu" class="hidden"></span></div>
      <table id="out-preview"></table>
    </section>
```

par :

```html
    <!-- Étape 2 : colonnes de sortie (aperçu manipulable) -->
    <section id="step-columns" class="panel hidden">
      <h2>Colonnes du fichier de sortie</h2>
      <p class="muted">Glisse les en-têtes : réordonne-les dans le tableau, écarte-les vers la
         zone du bas, réintègre-les où tu veux. L'aperçu montre le résultat final.</p>
      <table id="out-preview"></table>
      <div id="col-zone"></div>
    </section>
```

- [ ] **Step 2 : styles.css — supprimer les styles du ✕**

Supprimer les deux lignes (~73-74) :

```css
th .rm { color: var(--muted); cursor: pointer; margin-left: 6px; }
th .rm:hover { color: var(--red); }
```

- [ ] **Step 3 : styles.css — ajouter zone, chips, temp**

Sous le bloc « Drag des colonnes (SortableJS forceFallback)… » existant, ajouter :

```css
/* Drop zone des colonnes écartées : cible/départ du DnD (même langage
   visuel que le #dropzone de l'étape 1). */
#col-zone {
  border: 2px dashed var(--border); border-radius: 10px; padding: 10px 12px;
  display: flex; gap: 8px; flex-wrap: wrap; align-items: center; min-height: 48px;
}
#col-zone:empty::after {
  content: "Glisse ici les colonnes à écarter — et depuis ici pour les réintégrer.";
  color: var(--muted);
}
.chip {
  border: 1px solid var(--border); background: var(--card); border-radius: 14px;
  padding: 3px 12px; cursor: grab; user-select: none; white-space: nowrap;
}
.chip.peppol { color: var(--blue); border-color: var(--blue); }
#out-preview td.temp { background: rgba(88, 166, 255, .10); }
```

- [ ] **Step 4 : vérifier qu'aucun autre code ne référence l'ancien monde**

Run : `grep -rn "btn-add-col\|add-col-menu\|\.rm\b\|dragover" superpopaul/src --include="*.js" --include="*.html" --include="*.css"`
Attendu : seules occurrences restantes dans `columns.js` (supprimées en Task 3). Aucune dans cockpit.js/app.js.

---

### Task 3 : columns.js réécrit + nouveau défaut dans app.js

**Files:**
- Modify : `superpopaul/src/columns.js` (réécriture complète)
- Modify : `superpopaul/src/app.js:119-134` (bloc du défaut de colonnes)

**PAS de commit à la fin de cette tâche.**

- [ ] **Step 1 : remplacer intégralement columns.js**

Contenu complet du nouveau `superpopaul/src/columns.js` :

```js
// Étape 2 : le tableau de sortie AVEC données d'exemple est l'outil de
// configuration. Paradigme unique : le glisser-déposer — réordonner les
// colonnes dans le tableau, les écarter vers la drop zone (#col-zone), les
// réintégrer depuis la zone à l'emplacement voulu.
// Source de vérité : state.config.output.columns (la zone est calculée).

const PEPPOL_FIELDS = [
  ["exists", "existe"], ["pa_code", "code PA"], ["pa_name", "nom PA"],
  ["pa_country", "pays PA"], ["extended_ctc_fr", "CTC-FR"],
];
const PEPPOL_SAMPLE = { exists: "true", pa_code: "PA0042", pa_name: "ACME PA",
                        pa_country: "FR", extended_ctc_fr: "false" };

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
  return c.source === "input" ? c.name
       : "⚡ " + PEPPOL_FIELDS.find(([f]) => f === c.field)[1];
}

function makeHeader(c) {
  const attrs = { class: c.source, "data-key": colKey(c) };
  if (c.source === "peppol")
    attrs.title = "Champ calculé par l'API Peppol — les valeurs affichées sont un exemple.";
  return h("th", attrs, `⠿ ${colLabel(c)}`);
}

// Cellule du corps pour la colonne c et la ligne r du preview. `temp` marque
// une colonne matérialisée pendant un drag entrant (fond bleuté).
function makeCell(c, r, temp) {
  const key = colKey(c);
  if (c.source === "peppol")
    return h("td", { class: temp ? "muted temp" : "muted", "data-key": key },
      PEPPOL_SAMPLE[c.field]);
  const idx = state.preview.headers.indexOf(c.name);
  return h("td", { class: temp ? "temp" : "", "data-key": key },
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
    h("div", { class: `chip ${c.source}`, "data-key": colKey(c) }, `⠿ ${colLabel(c)}`)));
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
    onChange: syncBodyToHeaders,
    onEnd: () => setTimeout(commitFromHeaders, 0), // laisser Sortable clore son cycle
  };
  sortHead = new Sortable(head, {
    ...common,
    // Garde « minimum 1 colonne » : la dernière colonne refuse de partir.
    group: { name: "columns", pull: () => head.children.length > 1, put: true },
  });
  // sort: false — on drag vers/depuis la zone, jamais dedans : son ordre est
  // canonique au render et un tri manuel serait défait au re-render suivant.
  sortZone = new Sortable($("col-zone"), { ...common, group: "columns", sort: false });
}
```

- [ ] **Step 2 : app.js — nouveau défaut de colonnes**

Dans `app.js` (bloc lignes 119-134), remplacer :

```js
    // Mapping par défaut : toutes les colonnes d'entrée + les 4 champs Peppol.
```

par :

```js
    // Mapping par défaut : toutes les colonnes d'entrée + existe/CTC-FR ; les
    // autres champs Peppol démarrent dans la drop zone de l'étape 2.
```

et le tableau du défaut :

```js
      state.config.output.columns = [
        ...p.headers.map((name) => ({ source: "input", name })),
        { source: "peppol", field: "exists" },
        { source: "peppol", field: "pa_code" },
        { source: "peppol", field: "pa_country" },
        { source: "peppol", field: "extended_ctc_fr" },
      ];
```

par :

```js
      state.config.output.columns = [
        ...p.headers.map((name) => ({ source: "input", name })),
        { source: "peppol", field: "exists" },
        { source: "peppol", field: "extended_ctc_fr" },
      ];
```

- [ ] **Step 3 : contrôle statique rapide**

Run : `grep -n "btn-add-col\|add-col-menu\|renderAddColMenu\|\.rm\|data-idx" superpopaul/src/columns.js superpopaul/src/index.html superpopaul/src/styles.css`
Attendu : aucune occurrence.
Run : `node --check superpopaul/src/columns.js && node --check superpopaul/src/app.js`
Attendu : pas d'erreur de syntaxe.

---

### Task 4 : vérification navigateur (harnais + Playwright), puis commit frontend

**Files:**
- Create (scratchpad, non commité) : `<scratchpad>/harness-dropzone.html`
- Aucune modification de l'app sauf si un scénario échoue.

- [ ] **Step 1 : écrire le harnais**

`<scratchpad>/harness-dropzone.html` (le symlink `src` → `superpopaul/src` est recréé au Step 2) :

```html
<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<title>Harnais columns.js — drop zone</title>
<link rel="stylesheet" href="src/styles.css">
</head>
<body>
<main>
  <table id="out-preview"></table>
  <div id="col-zone"></div>
</main>
<script src="src/vendor/Sortable.min.js"></script>
<script>
function $(id) { return document.getElementById(id); }
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
const state = {
  preview: {
    // « exists » en 3e colonne d'ENTRÉE : teste la non-collision avec le
    // champ Peppol du même nom (clés préfixées input:/peppol:).
    headers: ["siren", "raison_sociale", "exists"],
    rows: [
      ["552 100 554", "ACME SAS", "oui"],
      ["317 218 758", "Dupont & Fils", "non"],
      ["833 456 129", "Boulangerie Lu", "oui"],
    ],
  },
  config: { output: { columns: [
    { source: "input", name: "siren" },
    { source: "input", name: "raison_sociale" },
    { source: "input", name: "exists" },
    { source: "peppol", field: "exists" },
    { source: "peppol", field: "extended_ctc_fr" },
  ] } },
};
</script>
<script src="src/columns.js"></script>
<script>renderOutPreview();</script>
</body>
</html>
```

- [ ] **Step 2 : servir et charger**

```bash
cd <scratchpad> && ln -sfn "$(git rev-parse --show-toplevel)/superpopaul/src" src \
  && python3 -m http.server 8935 --bind 127.0.0.1 &
```

Naviguer (Playwright MCP) vers `http://127.0.0.1:8935/harness-dropzone.html`.
Attendu : zéro erreur console (hors favicon 404) ; 5 th ; zone = 3 chips
(`⚡ code PA`, `⚡ nom PA`, `⚡ pays PA`).

- [ ] **Step 3 : dérouler les scénarios**

Piloter avec `page.mouse` (down / ≥ 12 moves espacés de ~30 ms / up, pauses
~100 ms après down et avant up — les drags trop rapides ratent l'émission des
événements Sortable, artefact de test connu). Helper de drag : celui des runs
précédents. Après chaque drop, lire `state.config.output.columns` (clés
`source:name/field`) et le DOM.

Scénarios et assertions :

1. **Réordonner** : th1 (`siren`) → position 3.
   `columns[2] == {source:"input", name:"siren"}` ; 1re ligne du corps réordonnée pareil.
2. **Écarter** : th `raison_sociale` → `#col-zone`.
   `columns.length == 4`, plus de `raison_sociale` ; une chip `⠿ raison_sociale` (sans classe `peppol`) dans la zone ; corps à 4 td par tr.
3. **Matérialiser + insérer** : chip `⚡ code PA` → entre th1 et th2, en échantillonnant pendant le drag.
   Pendant : `#out-preview td.temp` ≥ 1 (à un moment du survol).
   Après : `columns[1] == {source:"peppol", field:"pa_code"}` ; td `PA0042` en 2e cellule de la 1re ligne, sans classe `.temp` (re-render).
4. **Collision de noms** : la colonne d'entrée `exists` ET le champ Peppol `exists` coexistent : écarter le **th input** `exists` (data-key `input:exists`).
   Après : `columns` contient toujours `{source:"peppol", field:"exists"}`, plus `{source:"input", name:"exists"}` ; la zone a une chip `exists` NON-peppol ; le corps affiche encore la colonne Peppol (`true`).
5. **Garde min 1** : écarter des colonnes jusqu'à n'en garder qu'une, puis tenter d'écarter la dernière.
   Attendu : le drag ne démarre pas ou revert (`columns.length == 1` inchangé après la tentative).
6. **Annulation par spill** (amendé après vérif : Échap non supporté en
   forceFallback — spec mise à jour ; `revertOnSpill: true` requis dans le
   bloc `common` de columns.js) : glisser un th au-dessus de la zone puis
   relâcher HORS des deux listes (ex. marge de `<main>`).
   Attendu : `columns` inchangé ; corps resynchronisé (autant de td par tr que de th).
7. **Zone vide** : réintégrer toutes les chips une à une.
   Attendu : `#col-zone` sans enfant, texte d'aide visible (`getComputedStyle(zone, "::after").content` non vide) ; hauteur de zone ≥ 48 px.

- [ ] **Step 4 : capture visuelle**

Screenshot pendant un drag chip→tableau (matérialisation visible) et au repos
(zone + chips). Contrôle visuel : chips arrondies, zone pointillée, `td.temp`
bleuté.

- [ ] **Step 5 : nettoyage et commit frontend**

```bash
pkill -f "http.server 8935"; rm <scratchpad>/src
git add superpopaul/src/columns.js superpopaul/src/index.html \
        superpopaul/src/styles.css superpopaul/src/app.js
git commit -m "feat(superpopaul): étape 2 en DnD unique — drop zone de colonnes écartées

Le ✕ et le menu « + Ajouter une colonne » disparaissent : réordonner,
écarter et réintégrer se font au glisser-déposer entre le tableau
d'aperçu et une drop zone (deux listes Sortable, group commun). Le corps
suit l'ordre des en-têtes en continu pendant le drag (matérialisation
des colonnes entrantes avec leurs données). Nouveau défaut : colonnes
d'entrée + existe/CTC-FR ; code PA, pays PA et nom PA démarrent dans la
zone. Garde « minimum 1 colonne » côté UI, invariant repris par
Config::validate.

Spec : docs/superpowers/specs/2026-07-13-superpopaul-colonnes-dropzone-design.md"
```

---

## Self-review du plan (fait à l'écriture)

- **Couverture spec** : suppression ✕/menu (T2/T3), drop zone bandeau + aide (T2), défauts (T3.2), garde min 1 (T3.1 `pull` + scénario 5), zone `sort:false` (T3.1), annulation (scénario 6), invariant Rust (T1), nettoyage (T2.2, T3.3), vérification complète (T4). Rendu « 0 colonnes » supprimé : plus de branche `cols.length === 0` dans le nouveau columns.js ✓.
- **Placeholders** : aucun — chaque étape code porte son code complet.
- **Cohérence des noms** : `colKey`/`specFromKey`/`syncBodyToHeaders`/`commitFromHeaders`/`renderColZone`/`#col-zone`/`.chip`/`.temp` utilisés de façon identique entre T2, T3 et T4.
