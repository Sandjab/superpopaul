# Nom du profil & triptyque Ouvrir/Enregistrer/Enregistrer sous — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Afficher le profil actif (« nom • modifié ») dans la barre Format et remplacer les deux boutons profil par 📂 Ouvrir… / 💾 Enregistrer / Enregistrer sous…

**Architecture:** Frontend uniquement (spec `docs/superpowers/specs/2026-07-18-profil-nom-et-triptyque-design.md`) : état de session `state.profile = { path, name, ref }` où `ref` est un instantané JSON de `{pid_column, columns, encoding, separator}` ; « modifié » = divergence par comparaison, recalculée au rendu (hook global `window.updateProfileBar?.()` appelé en fin de `renderOutPreview`, motif `updateRunModeHint`). Zéro Rust.

**Tech Stack:** vanilla HTML/CSS/JS, helper `h()`, aucune couleur en dur.

**Conventions (CLAUDE.md projet) :** texte UI français ; jamais d'innerHTML avec données dynamiques ; commits `feat(superpopaul): …`.

**Mapping des ids** (décision structurante, à respecter partout) :
- `btn-load-cfg` = 📂 Ouvrir… (handler chargement existant, enrichi)
- `btn-save-cfg` = 💾 Enregistrer (NOUVEAU comportement : écrase sans dialogue ; grisé sans profil courant modifié)
- `btn-saveas-cfg` = Enregistrer sous… (l'ACTUEL comportement de sauvegarde avec dialogue ; grisé sans désignation)

---

### Task 1 : `index.html` + `styles.css` — barre Format

**Files:**
- Modify: `client/src/index.html` (bloc `#format-head`)
- Modify: `client/src/styles.css` (règle `#format-head`, nouvelles règles)

Transitoire assumé : entre cette task et la suivante, `app.js` attache l'ancien handler-dialogue au bouton 💾 — les Tasks 1-2 forment un tout, ne pas lancer l'app entre les deux.

- [ ] **Step 1 : Restructurer `#format-head` dans `index.html`**

Remplacer le bloc actuel (h2 + span des deux boutons) par :

```html
      <div id="format-head">
        <h2>Format du fichier principal</h2>
        <span id="profile-name" class="muted"></span>
        <span>
          <button id="btn-load-cfg" title="Ouvrir un profil YAML — appliqué si ses colonnes correspondent au fichier ouvert.">📂 Ouvrir…</button>
          <button id="btn-save-cfg" disabled title="Enregistrer — écrase le profil courant.">💾 Enregistrer</button>
          <button id="btn-saveas-cfg" title="Nouveau fichier YAML.">Enregistrer sous…</button>
        </span>
      </div>
```

(Le `disabled` initial sur 💾 reflète l'état de départ : aucun profil chargé.)

- [ ] **Step 2 : Styles**

Dans `styles.css`, remplacer la règle existante
`#format-head { display: flex; align-items: center; justify-content: space-between; }` par :

```css
#format-head { display: flex; align-items: center; gap: 10px; }
#format-head h2 { margin-right: auto; }
#profile-name { font-size: .85em; }
.profile-dirty { color: var(--gold); }
```

(`space-between` centrerait le nom entre titre et boutons ; `margin-right: auto`
sur le h2 pousse nom + boutons à droite, groupés. Le nom hérite sa couleur de
`.muted` ; « • modifié » prend l'or — rôle « activité » de l'identité, pas
l'orange d'avertissement.)

- [ ] **Step 3 : Vérification et commit**

Run : `grep -c 'id="btn-saveas-cfg"\|id="profile-name"' client/src/index.html` → 2 occurrences au total, et
`grep -oE 'id="[^"]+"' client/src/index.html | sort | uniq -d` → vide (pas de doublon).

```bash
git add client/src/index.html client/src/styles.css
git commit -m "feat(superpopaul): barre Format — nom du profil et triptyque Ouvrir/Enregistrer/Enregistrer sous"
```

---

### Task 2 : `app.js` + hook `columns.js` — état profil, instantané, handlers

**Files:**
- Modify: `client/src/app.js` (state, section profils, `renderPidSelect`, `pickInput`)
- Modify: `client/src/columns.js` (une ligne en fin de `renderOutPreview`)

- [ ] **Step 1 : État**

Dans l'objet `state` (en tête d'app.js), ajouter après `preview: null, …` :

```js
  // Profil courant (session seulement) : chemin/nom du YAML et instantané de
  // référence (profileSnapshot) — null tant qu'aucun profil chargé/enregistré.
  profile: null, // { path, name, ref }
```

- [ ] **Step 2 : Instantané, payload et barre — nouvelles fonctions**

Dans la section « Profils de chargement » (après `profileDialogDefault`), ajouter :

```js
/** Empreinte de l'état que porte un profil. La référence (`state.profile.ref`)
 *  est prise au chargement et à chaque enregistrement réussi ; « modifié » =
 *  divergence par comparaison — aucun point de mutation à instrumenter. */
function profileSnapshot() {
  const c = state.config;
  return JSON.stringify({ pid: c.input.pid_column, columns: c.output.columns,
                          encoding: c.output.encoding, separator: c.output.separator });
}

/** Le payload envoyé à save_profile — partagé par Enregistrer et
 *  Enregistrer sous… (la validation vit côté Rust, Profile::validate). */
function currentProfilePayload() {
  return {
    version: 1,
    input: { pid_column: state.config.input.pid_column,
             columns_hash: state.preview.columns_hash },
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
```

- [ ] **Step 3 : Handlers du triptyque**

L'actuel handler `$("btn-save-cfg")` (sauvegarde AVEC dialogue) devient celui de
`btn-saveas-cfg`, avec trois enrichissements : payload factorisé, proposition du
nom courant, adoption du fichier choisi comme profil courant :

```js
$("btn-saveas-cfg").addEventListener("click", async () => {
  const dflt = await profileDialogDefault();
  // Propose le nom du profil courant comme point de départ (dans le dossier
  // portable le cas échéant).
  if (state.profile)
    dflt.defaultPath = dflt.defaultPath
      ? `${dflt.defaultPath}/${state.profile.name}` : state.profile.name;
  const f = await save({ filters: [{ name: "YAML", extensions: ["yaml", "yml"] }], ...dflt });
  if (!f) return;
  try {
    await invoke("save_profile", { path: f, profile: currentProfilePayload() });
    state.profile = { path: f, name: f.split(/[\\/]/).pop() ?? f, ref: profileSnapshot() };
    hideBanner();
  } catch (e) {
    banner("error", `${e}`);
  }
  renderProfileBar();
});
```

Nouveau handler `btn-save-cfg` (💾, sans dialogue — n'est cliquable que si un
profil courant existe et diverge, cf. `renderProfileBar`) :

```js
$("btn-save-cfg").addEventListener("click", async () => {
  try {
    await invoke("save_profile", { path: state.profile.path, profile: currentProfilePayload() });
    state.profile.ref = profileSnapshot();
    hideBanner();
  } catch (e) {
    banner("error", `${e}`);
  }
  renderProfileBar();
});
```

Handler `btn-load-cfg` : après le bloc d'application de l'état (les quatre
affectations `state.config…`), et avant `hideBanner()`, ajouter :

```js
  state.profile = { path: f, name: f.split(/[\\/]/).pop() ?? f, ref: profileSnapshot() };
```

et ajouter `renderProfileBar();` en fin de handler (après `syncStepperGating();`).

- [ ] **Step 4 : Grisage d'Enregistrer sous… et cycle de vie au dépôt**

Dans `renderPidSelect()`, la ligne
`$("btn-save-cfg").disabled = !state.config.input.pid_column;` devient :

```js
  // Un profil sans désignation serait invalide : « Enregistrer sous… » grisé.
  $("btn-saveas-cfg").disabled = !state.config.input.pid_column;
```

(le grisage de 💾 appartient à `renderProfileBar`).

Dans `pickInput()`, juste après le bloc `if (state.config.output.columns.length === 0 || headersChanged) { … }` :

```js
    // Le contexte profil ne survit pas à un changement de signature de
    // colonnes : le profil chargé ne décrit plus ce fichier.
    if (state.profile && headersChanged) state.profile = null;
```

et ajouter `renderProfileBar();` juste après l'appel `renderFilePanel();`.

Enfin, les changements d'encodage/séparateur ne passent pas par
`renderOutPreview` (donc pas par le hook) : leurs deux listeners doivent
rafraîchir la barre eux-mêmes. Dans le bloc « Étape Format : forme de
sortie », les deux listeners deviennent :

```js
$("out-encoding").addEventListener("change", (e) => { state.config.output.encoding = e.target.value; renderProfileBar(); });
$("out-sep").addEventListener("change", (e) => { state.config.output.separator = e.target.value; renderProfileBar(); });
```

- [ ] **Step 5 : Hook côté `columns.js`**

En toute fin de `renderOutPreview()` (après la création de `sortZone`), ajouter :

```js
  // Toute manipulation du tableau peut faire diverger l'état du profil
  // courant — app.js recalcule la barre (nom • modifié, grisage de 💾).
  window.updateProfileBar?.();
```

- [ ] **Step 6 : Vérification statique et commit**

Run :
- `node --check client/src/app.js && node --check client/src/columns.js` → propre.
- `grep -n "btn-save-cfg\|btn-saveas-cfg\|profile-name\|updateProfileBar" client/src/app.js client/src/columns.js` → coller la sortie ; vérifier que `btn-save-cfg` n'apparaît plus que dans `renderProfileBar` et son handler 💾.

```bash
git add client/src/app.js client/src/columns.js
git commit -m "feat(superpopaul): profil courant en session — nom • modifié, Enregistrer direct, Enregistrer sous"
```

---

### Task 3 : Vérification de bout en bout

**Files:** aucun (vérification) — corrections éventuelles au fil de l'eau.

- [ ] **Step 1 : Statique**

Run : `cd client/src-tauri && cargo test 2>&1 | grep "test result"` (rien ne doit
avoir bougé : 187 passed) ; `node --check` sur les trois JS ;
`grep -oE 'id="[^"]+"' client/src/index.html | sort | uniq -d` → vide.

- [ ] **Step 2 : Vérification manuelle (avec l'utilisateur)**

1. Fichier déposé, aucune ouverture de profil : barre sans nom, 💾 grisé,
   « Enregistrer sous… » actif dès désignation.
2. Enregistrer sous… → le nom apparaît (sans « • modifié »), 💾 grisé.
3. Modifier (drag une colonne, ou l'encodage) → « • modifié » doré apparaît,
   💾 s'active ; 💾 → l'indicateur disparaît, le YAML sur disque est à jour.
4. Ouvrir… un profil compatible → nom posé, état propre ; incompatible →
   refus sec inchangé, barre inchangée.
5. Re-dépôt du même fichier → contexte conservé ; dépôt d'un fichier à
   colonnes différentes → nom effacé, 💾 grisé.
6. Enregistrer sous… avec profil courant → le dialogue propose son nom.

- [ ] **Step 3 : Commit final si corrections**

```bash
git add -A && git commit -m "fix(superpopaul): ajustements vérification triptyque profil"
```

(Seulement s'il y a eu des corrections.)
