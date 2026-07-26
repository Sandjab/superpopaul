# Ajout de comptes par run, superposition, export XLSX — plan d'implémentation

> **Pour les agents :** SOUS-SKILL REQUIS — utiliser `superpowers:subagent-driven-development`
> (recommandé) ou `superpowers:executing-plans` pour exécuter ce plan tâche par
> tâche. Les étapes sont en cases à cocher (`- [ ]`).

**But :** rendre la fenêtre d'ajout visible, faire du Run de Facturation le point
d'entrée de l'ajout de comptes, et produire un classeur XLSX du périmètre complet.

**Architecture :** trois volets indépendants. (A) une règle CSS et son test
d'ordre des couches. (B) une commande Rust `plan_candidats_run` qui filtre par
jour de cycle, une fenêtre JS qui trie et filtre en mémoire. (C) un module
`plan_xlsx` séparant la composition du tableau (pure, testable) de son écriture
(I/O, `rust_xlsxwriter`).

**Pile :** Rust 2021, Tauri 2, `rust_xlsxwriter` 0.93. Frontend vanilla, sans
bundler ni framework. Tests : `cargo test` depuis `client/src-tauri/`,
`node --test "tests/*.test.js"` depuis `client/`.

**Spec :** `docs/superpowers/specs/2026-07-26-ajout-comptes-par-run-design.md`
**Maquette validée :** `docs/superpowers/maquettes/2026-07-26-ajout-comptes-par-run.html`

---

## Contexte indispensable avant de commencer

Lire la spec. Les points qu'on oublie sinon :

- **Texte, commentaires et noms de tests en français.** Seule exception, décidée
  explicitement : les valeurs de statut CTC restent brutes (`ready`, `later`,
  `expired`, vide) y compris à l'écran — décision 9 de la spec, à ne pas
  « corriger ».
- **Jamais d'`innerHTML` avec des données dynamiques** côté JS (contenu CSV,
  messages d'erreur backend) : construire le DOM via le helper `h()` de
  `app.js` ou `textContent`. Un CSV est une entrée non fiable.
- **`esc` obligatoire** côté Rust sur toute donnée d'origine CSV ou SMP.
- **Modules purs** : `plan_xlsx::lignes` n'a ni DB, ni UI, ni disque.
- **TDD** : le test d'abord, on le voit échouer, puis le code minimal.
- **Une chaîne cherchée dans un document riche a presque toujours plus d'un
  producteur.** Le lot précédent a livré 15 tests incapables d'échouer pour
  cette raison. Avant de figer une assertion, vérifier que la chaîne visée n'a
  qu'une seule source possible.

Commandes de test :

```bash
cd client/src-tauri && cargo test          # Rust
cd client && node --test "tests/*.test.js" # JS
```

État de départ attendu : **497 tests Rust**, **29 tests JS**, 5 warnings clippy
préexistants (`commands.rs:88`, `direct.rs:108`, `direct.rs:560`,
`directory.rs:47`, `resolver.rs:142`). Le vérifier avant de commencer ; si le
compte diffère, s'arrêter et le signaler.

Pour compter les warnings clippy, ne retenir que les lignes `warning: ` suivies
d'une ligne `-->` : les lignes de résumé « generated N warnings » font
surcompter.

---

## Structure des fichiers

| Fichier | Responsabilité |
|---------|----------------|
| `client/src/styles.css` | couches d'empilement, `.modal-wide`, styles de la liste et de l'action timeline |
| `client/src/app.js` | action par run, fenêtre de choix (tri + filtres), retrait du bouton global |
| `client/src-tauri/src/plan_xlsx.rs` | **neuf** — composition (pure) et écriture du classeur |
| `client/src-tauri/src/commands.rs` | `plan_candidats_run`, `Candidat` enrichi, `ctc_status`, branchement XLSX |
| `client/src-tauri/src/plan.rs` | `LigneEntree.ctc_status`, `parse_jj` exposée |
| `client/src-tauri/src/lib.rs` | module `plan_xlsx`, `invoke_handler` |
| `client/src-tauri/Cargo.toml` | dépendance `rust_xlsxwriter` |
| `client/tests/plan_ajout_run.test.js` | **neuf** — tri, filtres, couches |

---

# VOLET A — la fenêtre passe sous l'écran du plan

## Tâche 1 : remonter la modale au-dessus de l'écran plein

**Fichiers :**
- Modifier : `client/src/styles.css:377`
- Créer : `client/tests/plan_ajout_run.test.js`

- [ ] **Étape 1 : écrire le test qui échoue**

Créer `client/tests/plan_ajout_run.test.js` :

```js
// Ajout de comptes par run : couches d'empilement, tri et filtres.
//
// Vécu en application : la fenêtre d'ajout ouverte depuis le Plan de charge
// était invisible — l'écran plein du plan la recouvrait. Il fallait la fermer
// pour voir ce qu'elle affichait.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const CSS = fs.readFileSync(path.join(__dirname, "..", "src", "styles.css"), "utf8");

/** Le z-index déclaré pour un sélecteur, tel qu'écrit dans la feuille. */
function couche(selecteur) {
  const bloc = CSS.split(selecteur + " {")[1];
  assert.ok(bloc, `sélecteur « ${selecteur} » introuvable dans styles.css`);
  const m = bloc.split("}")[0].match(/z-index:\s*(\d+)/);
  assert.ok(m, `pas de z-index sur « ${selecteur} »`);
  return Number(m[1]);
}

test("la modale s'empile au-dessus de l'écran plein du plan", () => {
  // On teste l'ORDRE, pas les valeurs : un test sur « z-index: 70 » casserait
  // au prochain réglage sans rien signaler d'utile.
  const settings = couche("#settings-backdrop");
  const plan = couche("#plan-screen");
  const modale = couche("#modal-backdrop");
  const splash = couche("#splash");

  assert.ok(settings < plan, `réglages (${settings}) doit rester sous l'écran du plan (${plan})`);
  assert.ok(plan < modale, `l'écran du plan (${plan}) doit rester sous la modale (${modale})`);
  assert.ok(modale < splash, `la modale (${modale}) doit rester sous le splash (${splash})`);
});
```

- [ ] **Étape 2 : lancer le test et vérifier qu'il échoue**

```bash
cd client && node --test "tests/plan_ajout_run.test.js"
```

Attendu : échec sur `l'écran du plan (60) doit rester sous la modale (50)`.

- [ ] **Étape 3 : corriger la couche**

Dans `client/src/styles.css`, remplacer la règle `#modal-backdrop` :

```css
/* Au-dessus de #plan-screen (60), qui est un écran plein : sinon toute modale
   ouverte depuis le Plan de charge est recouverte et paraît ne pas s'ouvrir.
   Reste sous #splash (99). #settings-backdrop (40) reste dessous, pour que la
   saisie des identifiants proxy s'empile par-dessus les réglages. */
#modal-backdrop {
  position: fixed; inset: 0; background: rgba(0,0,0,.6); z-index: 70;
  display: flex; align-items: center; justify-content: center;
}
```

Adapter aussi le commentaire de `#settings-backdrop` (`styles.css:355-356`), qui
affirme « sous la modale (z-index 40 < 50) » : la valeur citée devient fausse.
Remplacer par « sous la modale (40 < 70) ».

- [ ] **Étape 4 : lancer le test et vérifier qu'il passe**

```bash
cd client && node --test "tests/plan_ajout_run.test.js"
```

Attendu : `pass 1`.

- [ ] **Étape 5 : commit**

```bash
git add client/src/styles.css client/tests/plan_ajout_run.test.js
git commit -m "fix(superpopaul): la fenêtre d'ajout ne passe plus sous l'écran du plan"
```

---

# VOLET B — le run devient le point d'entrée

## Tâche 2 : conserver le statut CTC complet

Le statut est calculé puis **aplati** en booléen. On garde la chaîne.

**Fichiers :**
- Modifier : `client/src-tauri/src/plan.rs:21-39`
- Modifier : `client/src-tauri/src/commands.rs:816`

- [ ] **Étape 1 : écrire le test qui échoue**

Dans `plan.rs`, `mod tests`, ajouter :

```rust
#[test]
fn le_statut_ctc_nest_pas_aplati_en_booleen() {
    // « later » et « expired » sont deux situations distinctes qu'un booléen
    // `ctc_ready == false` confond : on arbitre différemment sur l'une et
    // sur l'autre.
    let e = LigneEntree {
        cf: "CF1".into(),
        participant: "0225:1".into(),
        jj_brut: "5".into(),
        raison_sociale: "ACME".into(),
        pa: "Cegedim".into(),
        resolu: true,
        ctc_ready: false,
        ctc_status: "later".into(),
        ppf_usable: true,
        in_directory: true,
        resolved_at: 0,
    };
    assert_eq!(e.ctc_status, "later");
    assert!(!e.ctc_ready, "« later » n'est pas « ready »");
}
```

- [ ] **Étape 2 : lancer le test et vérifier qu'il échoue**

```bash
cd client/src-tauri && cargo test le_statut_ctc_nest_pas_aplati
```

Attendu : `struct LigneEntree has no field named ctc_status`.

- [ ] **Étape 3 : ajouter le champ**

Dans `plan.rs`, structure `LigneEntree`, après `ctc_ready` :

```rust
    /// Statut CTC complet : `"ready"` | `"later"` | `"expired"` | `""`.
    /// `ctc_ready` en est l'aplatissement et reste consommé par
    /// `construire_pool` et le funnel ; on conserve la chaîne parce que
    /// « prêt plus tard » et « expiré » ne s'arbitrent pas de la même façon.
    pub ctc_status: String,
```

- [ ] **Étape 4 : alimenter le champ**

Dans `commands.rs`, remplacer la ligne qui calcule `ctc_ready` (vers 816) :

```rust
        let ctc_status = r.map(|r| output::ctc_status(r, now)).unwrap_or("").to_string();
        let ctc_ready = ctc_status == "ready";
```

Puis ajouter `ctc_status,` au littéral `crate::plan::LigneEntree { … }` qui suit.

- [ ] **Étape 5 : réparer les autres constructions**

`cargo build` signale toute autre construction de `LigneEntree` (tests
compris). Pour chacune, ajouter le champ avec une valeur cohérente :
`"ready".into()` là où `ctc_ready: true`, `"".into()` sinon. **Ne pas modifier
l'intention d'un test existant** au passage.

- [ ] **Étape 6 : lancer la suite et commit**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -3
git add client/src-tauri/src/plan.rs client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): conserver le statut CTC complet des entrées"
```

Attendu : 498 tests verts (497 + 1).

## Tâche 3 : `plan_candidats_run`

**Fichiers :**
- Modifier : `client/src-tauri/src/plan.rs:156` (exposer `parse_jj`)
- Modifier : `client/src-tauri/src/commands.rs:1161` (`Candidat`), `1234` (commande)
- Modifier : `client/src-tauri/src/lib.rs:99` (`invoke_handler`)

- [ ] **Étape 1 : exposer `parse_jj`**

Dans `plan.rs`, la fonction est privée et `plan_candidats` en refait une copie
inline. La copie disparaît avec la commande ; on expose l'originale plutôt que
d'en écrire une troisième :

```rust
pub(crate) fn parse_jj(brut: &str) -> Option<u8> {
```

- [ ] **Étape 2 : écrire les tests qui échouent**

Dans `commands.rs`, `mod tests` (le créer s'il n'existe pas, avec
`#[cfg(test)] mod tests { use super::*; }`), ajouter :

```rust
    /// Un run couvrant les jours de cycle 1 et 5.
    fn run_test() -> crate::calendrier::RunFacturation {
        crate::calendrier::RunFacturation {
            num: "R3".into(),
            date: chrono::NaiveDate::from_ymd_opt(2026, 9, 8).unwrap(),
            jjs: vec![1, 5],
            exclu: false,
        }
    }

    fn entree(cf: &str, jj: &str, ctc: &str, ppf: bool) -> crate::plan::LigneEntree {
        crate::plan::LigneEntree {
            cf: cf.into(),
            participant: "0225:1".into(),
            jj_brut: jj.into(),
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            resolu: true,
            ctc_ready: ctc == "ready",
            ctc_status: ctc.into(),
            ppf_usable: ppf,
            in_directory: true,
            resolved_at: 0,
        }
    }

    #[test]
    fn candidats_run_ne_rend_que_les_jours_de_cycle_couverts() {
        let entrees = vec![
            entree("CF1", "5", "ready", true),
            entree("CF2", "12", "ready", true),
        ];
        let out = candidats_du_run(&entrees, &run_test(), &HashSet::new());
        assert_eq!(out.len(), 1, "le jour 12 n'est pas couvert par ce run");
        assert_eq!(out[0].cf, "CF1");
    }

    #[test]
    fn candidats_run_exclut_les_comptes_deja_au_plan() {
        let entrees = vec![entree("CF1", "5", "ready", true)];
        let deja: HashSet<String> = ["CF1".to_string()].into_iter().collect();
        assert!(candidats_du_run(&entrees, &run_test(), &deja).is_empty());
    }

    #[test]
    fn candidats_run_rend_les_non_eligibles_signales() {
        // Les forcer reste un choix assumé : ils sont proposés ET marqués.
        let entrees = vec![
            entree("CF1", "5", "later", true),
            entree("CF2", "1", "ready", false),
        ];
        let out = candidats_du_run(&entrees, &run_test(), &HashSet::new());
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| !c.eligible), "aucun des deux n'est pleinement éligible");
    }

    #[test]
    fn candidats_run_porte_le_statut_ctc_complet() {
        // Le test qui distingue le champ neuf du booléen préexistant : sans lui,
        // rendre `ctc_status` toujours vide passerait inaperçu.
        let entrees = vec![
            entree("CF1", "5", "later", true),
            entree("CF2", "1", "expired", true),
        ];
        let out = candidats_du_run(&entrees, &run_test(), &HashSet::new());
        let statuts: Vec<&str> = out.iter().map(|c| c.ctc_status.as_str()).collect();
        assert!(statuts.contains(&"later") && statuts.contains(&"expired"), "{statuts:?}");
    }

    #[test]
    fn candidats_run_ignore_un_jour_de_cycle_illisible() {
        // Un JJ hors bornes ou non numérique ne correspond à aucun run.
        let entrees = vec![entree("CF1", "zzz", "ready", true), entree("CF2", "99", "ready", true)];
        assert!(candidats_du_run(&entrees, &run_test(), &HashSet::new()).is_empty());
    }
```

- [ ] **Étape 3 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client/src-tauri && cargo test candidats_run
```

Attendu : `cannot find function 'candidats_du_run' in this scope`.

- [ ] **Étape 4 : enrichir `Candidat` et écrire la fonction pure**

Dans `commands.rs`, remplacer la structure `Candidat` :

```rust
#[derive(serde::Serialize)]
pub struct Candidat {
    pub cf: String,
    pub raison_sociale: String,
    pub jj: u8,
    pub pa: String,
    /// Agrégat qui décide du marquage ⚠ : CTC prêt ET PPF utilisable.
    pub eligible: bool,
    /// Adressage sous forme nue (`0225:…`) quand le schéma s'y prête.
    pub participant: String,
    /// `"ready"` | `"later"` | `"expired"` | `""` — jamais aplati.
    pub ctc_status: String,
    pub ppf_usable: bool,
}
```

Conserver les dérivations déjà présentes sur la structure d'origine si elles
diffèrent de `serde::Serialize` seul.

Ajouter la fonction pure, juste avant la commande :

```rust
/// Comptes proposables sur un run : jour de cycle couvert et absents du plan.
///
/// Le filtre par jour de cycle est une contrainte arithmétique — un run ne peut
/// pas facturer un autre jour. En revanche un compte non éligible (CTC non prêt,
/// PPF non utilisable) est proposé et **signalé** : le forcer est un choix assumé.
fn candidats_du_run(
    entrees: &[crate::plan::LigneEntree],
    run: &crate::calendrier::RunFacturation,
    deja_au_plan: &HashSet<String>,
) -> Vec<Candidat> {
    entrees
        .iter()
        .filter(|e| !deja_au_plan.contains(&e.cf))
        .filter_map(|e| {
            let jj = crate::plan::parse_jj(&e.jj_brut)?;
            run.couvre(jj).then(|| Candidat {
                cf: e.cf.clone(),
                raison_sociale: e.raison_sociale.clone(),
                jj,
                pa: e.pa.clone(),
                eligible: e.ctc_ready && e.ppf_usable,
                participant: crate::directory::parse_0225_value(&e.participant)
                    .unwrap_or_else(|| e.participant.clone()),
                ctc_status: e.ctc_status.clone(),
                ppf_usable: e.ppf_usable,
            })
        })
        .collect()
}
```

⚠️ **Vérifier la signature réelle de `directory::parse_0225_value`** avant de
l'utiliser : le plan suppose qu'elle rend `Option<String>`. Si elle rend autre
chose, adapter la conversion — ne pas modifier la fonction.

- [ ] **Étape 5 : lancer les tests et vérifier qu'ils passent**

```bash
cd client/src-tauri && cargo test candidats_run
```

Attendu : 5 tests `ok`.

- [ ] **Étape 6 : remplacer la commande**

Supprimer `plan_candidats` et écrire à sa place :

```rust
/// Comptes proposables sur un run donné. Le run est le point d'entrée : on
/// choisit d'abord où livrer, ensuite quoi y mettre.
#[tauri::command]
pub async fn plan_candidats_run(
    state: State<'_, AppState>,
    run_num: String,
) -> Result<Vec<Candidat>, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (_, meta) = charger_pour_retouche(&store)?;
        let (runs, _) = calendrier_du_plan(&meta)?;
        // Run inconnu ou écarté : une erreur nommée, pas une liste vide qui
        // ferait croire qu'aucun compte n'est proposable.
        let run = runs
            .iter()
            .find(|r| r.num == run_num)
            .ok_or_else(|| format!("run « {run_num} » inconnu ou écarté du plan"))?;
        let deja: HashSet<String> = {
            let s = store.lock().unwrap();
            s.plan_lignes()?.into_iter().map(|l| l.cf).collect()
        };
        let entrees = entrees_par_cf(&store, &input, &cfg)?;
        Ok(candidats_du_run(&entrees, run, &deja))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

⚠️ Les deux lignes qui obtiennent `deja` et `entrees` sont **reprises de
l'ancienne `plan_candidats`** (`commands.rs:1239-1246`) : lire son corps réel et
transposer ses appels exacts plutôt que ceux écrits ici, qui sont une
reconstitution. Le reste de la commande est neuf.

**Trou de couverture assumé, à signaler dans le compte rendu :** la garde « run
inconnu ou écarté » vit dans la commande Tauri, qui prend un `State` et ne se
teste pas sans simuler tout le contexte de l'application. La spec prévoyait
`candidats_run_refuse_un_run_inconnu` ; il n'est pas écrit ici. Le cas se
vérifie au **parcours GUI** (tâche 10). Ne pas l'oublier au motif qu'aucun test
ne le rappelle.

- [ ] **Étape 7 : mettre à jour l'`invoke_handler`**

Dans `client/src-tauri/src/lib.rs`, remplacer `commands::plan_candidats,` par
`commands::plan_candidats_run,`. Une commande sans appelant est du code mort.

- [ ] **Étape 8 : compiler, tester, commit**

```bash
cd client/src-tauri && cargo build && cargo test 2>&1 | tail -3
git add client/src-tauri/src/
git commit -m "feat(superpopaul): candidats à l'ajout filtrés par run"
```

## Tâche 4 : l'action « + Ajouter » dans la timeline

**Fichiers :**
- Modifier : `client/src/app.js:1585-1607`
- Modifier : `client/src/styles.css`

- [ ] **Étape 1 : ajouter la colonne d'action**

Dans `app.js`, la fonction qui rend une ligne de timeline construit `boite` (la
case « exclure ») puis renvoie soit une ligne écartée, soit une ligne de run.
Ajouter avant le `return` de la ligne de run :

```js
  // Le run est le point d'entrée de l'ajout : l'action vit sur sa ligne.
  const ajout = h("td", { class: "tl-add" },
    h("button", { class: "tl-add-btn", onclick: () => ouvrirAjoutRun(r) }, "+ Ajouter"));
```

Passer `ajout` en dernier argument du `h("tr", { class: "tl-run" }, …)`.

Pour la ligne écartée, ajouter une cellule vide en dernier argument — un run
écarté ne porte pas l'action, on ne peut rien y placer :

```js
      h("td", { class: "tl-add" }));
```

- [ ] **Étape 2 : ajuster l'en-tête de la table**

La table `plan-tl` gagne une colonne. Trouver la ligne d'en-têtes
(`app.js:1644`, `h("table", { class: "plan-tl" }, …)`) et lui ajouter un
`h("th", {})` final. Vérifier que le `colspan: "5"` de la ligne écartée
(`app.js:1592`) reste cohérent : il couvre les colonnes de chiffres, pas la
case d'exclusion ni la nouvelle colonne — **le compter sur le rendu réel** plutôt
que de supposer.

- [ ] **Étape 3 : styles**

Ajouter à `client/src/styles.css`, à la suite des règles `plan-tl` :

```css
/* Le run est le point d'entrée de l'ajout : action discrète au repos, dorée
   au survol de la ligne — elle ne doit pas concurrencer les chiffres. */
td.tl-add { text-align: right; width: 1%; }
.tl-add-btn {
  background: none; border: 1px solid var(--border); border-radius: 6px;
  color: var(--muted); cursor: pointer; font-size: 11.5px; padding: 1px 8px;
  white-space: nowrap;
}
tr.tl-run:hover .tl-add-btn { color: var(--gold); border-color: var(--gold); }
```

- [ ] **Étape 4 : retirer le bouton global**

Dans `app.js:1849`, supprimer de la barre d'outils du récap :

```js
    h("button", { class: "btn-primary", onclick: ouvrirAjout }, "+ Ajouter des comptes…"),
```

Supprimer ensuite la fonction `ouvrirAjout` devenue orpheline (elle sera
remplacée par `ouvrirAjoutRun` à la tâche 5). Vérifier qu'aucun autre appelant
ne subsiste : `grep -n "ouvrirAjout\b" client/src/app.js`.

- [ ] **Étape 5 : commit**

```bash
git add client/src/app.js client/src/styles.css
git commit -m "feat(superpopaul): l'ajout de comptes part du run dans la timeline"
```

## Tâche 5 : la fenêtre de choix, triable et filtrable

**Fichiers :**
- Modifier : `client/src/app.js`
- Modifier : `client/src/styles.css`
- Modifier : `client/tests/plan_ajout_run.test.js`

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter à `client/tests/plan_ajout_run.test.js` :

```js
const { chargerApp } = require("./dom_shim");

/** Trois candidats aux statuts contrastés, tels que `plan_candidats_run` les rend. */
const CANDIDATS = [
  { cf: "CF-A", raison_sociale: "ALPHA SARL", jj: 5, pa: "Cegedim",
    eligible: true, participant: "0225:1", ctc_status: "ready", ppf_usable: true },
  { cf: "CF-B", raison_sociale: "BETA SAS", jj: 1, pa: "SAGE",
    eligible: false, participant: "0225:2", ctc_status: "later", ppf_usable: true },
  { cf: "CF-C", raison_sociale: "GAMMA SCI", jj: 5, pa: "Cegedim",
    eligible: false, participant: "0225:3", ctc_status: "ready", ppf_usable: false },
];

test("le tri par colonne réordonne la liste", () => {
  const ctx = chargerApp();
  const lignes = ctx.trierCandidats(CANDIDATS, "cf", false).map((c) => c.cf);
  assert.deepEqual(lignes, ["CF-C", "CF-B", "CF-A"], "tri descendant sur le compte");
});

test("les filtres se combinent", () => {
  const ctx = chargerApp();
  // Plateforme ET statut CTC actifs en même temps : un filtre seul laisserait
  // passer CF-C (Cegedim, ready) ou CF-B (later).
  const out = ctx.filtrerCandidats(CANDIDATS, { texte: "", pa: "Cegedim", ctc: "ready", ppf: "" });
  assert.deepEqual(out.map((c) => c.cf), ["CF-A", "CF-C"]);
  const strict = ctx.filtrerCandidats(CANDIDATS, { texte: "", pa: "Cegedim", ctc: "ready", ppf: "oui" });
  assert.deepEqual(strict.map((c) => c.cf), ["CF-A"], "le filtre PPF doit encore réduire");
});

test("la recherche porte sur le compte et la raison sociale", () => {
  const ctx = chargerApp();
  assert.deepEqual(
    ctx.filtrerCandidats(CANDIDATS, { texte: "beta", pa: "", ctc: "", ppf: "" }).map((c) => c.cf),
    ["CF-B"]);
  assert.deepEqual(
    ctx.filtrerCandidats(CANDIDATS, { texte: "cf-c", pa: "", ctc: "", ppf: "" }).map((c) => c.cf),
    ["CF-C"], "la recherche ignore la casse");
});
```

⚠️ **`chargerApp` doit exposer `trierCandidats` et `filtrerCandidats`.** Lire
`client/tests/dom_shim.js` pour savoir comment il expose les fonctions de
`app.js` (les tests existants s'en servent déjà) et suivre le même mécanisme.
Si le shim n'expose que ce qui est attaché à un objet global, attacher ces deux
fonctions de la même façon que les fonctions déjà testées.

- [ ] **Étape 2 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client && node --test "tests/plan_ajout_run.test.js"
```

Attendu : `ctx.trierCandidats is not a function`.

- [ ] **Étape 3 : écrire les deux fonctions pures**

Dans `app.js`, à côté des autres helpers du plan :

```js
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
```

- [ ] **Étape 4 : lancer les tests et vérifier qu'ils passent**

```bash
cd client && node --test "tests/plan_ajout_run.test.js"
```

Attendu : 4 tests `pass`.

- [ ] **Étape 5 : écrire la fenêtre**

Remplacer l'ancienne `ouvrirAjout` par `ouvrirAjoutRun(run)`. La maquette
`docs/superpowers/maquettes/2026-07-26-ajout-comptes-par-run.html` donne le
balisage cible, classe par classe.

**Construire le DOM avec `h()` uniquement** : les raisons sociales viennent du
CSV, jamais d'`innerHTML`.

```js
/** Pastille de statut : valeurs brutes assumées (décision 9 de la spec). */
function pastilleCtc(s) {
  const classe = { ready: "st-ready", later: "st-later", expired: "st-expired" }[s] ?? "st-none";
  return h("span", { class: `st ${classe}` }, s || "(vide)");
}

async function ouvrirAjoutRun(run) {
  let candidats;
  try {
    candidats = await invoke("plan_candidats_run", { runNum: run.num });
  } catch (e) {
    return planBanner("error", String(e));
  }

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

  function dessiner() {
    const vus = trierCandidats(filtrerCandidats(candidats, filtres), tri.colonne, tri.croissant);
    const toutCoche = h("input", { type: "checkbox", onchange: (ev) => {
      for (const c of vus) ev.target.checked ? choisis.add(c.cf) : choisis.delete(c.cf);
      dessiner();
    } });
    corps.replaceChildren(h("table", { class: "plan-data" },
      h("tr", {},
        h("th", { style: "width:1%" }, toutCoche),
        enTete("cf", "Compte"), enTete("raison_sociale", "Raison sociale"),
        enTete("jj", "JJ", "n"), enTete("pa", "Plateforme"),
        enTete("ctc_status", "CTC"), enTete("ppf_usable", "PPF")),
      ...vus.map((c) => h("tr", { class: `${c.eligible ? "" : "warn"} ${choisis.has(c.cf) ? "sel" : ""}`.trim() },
        h("td", {}, h("input", { type: "checkbox", checked: choisis.has(c.cf), onchange: (ev) => {
          ev.target.checked ? choisis.add(c.cf) : choisis.delete(c.cf);
          dessiner();
        } })),
        h("td", { class: "cf" }, c.cf),
        h("td", {}, c.raison_sociale),
        h("td", { class: "n jj" }, String(c.jj)),
        h("td", { class: "pa" }, c.pa),
        h("td", {}, pastilleCtc(c.ctc_status)),
        h("td", {}, h("span", { class: `st ${c.ppf_usable ? "st-yes" : "st-no"}` },
          String(c.ppf_usable)))))));
    const forces = [...choisis].filter((cf) => !candidats.find((c) => c.cf === cf)?.eligible).length;
    pied.replaceChildren(
      h("b", {}, String(choisis.size)), ` compte(s) sélectionné(s)`,
      ...(forces ? [" · ", h("span", { class: "warn-n" }, `${forces} non pleinement éligible(s)`)] : []),
      h("br", {}),
      h("span", { style: "font-size:12px" },
        `${fmtN(candidats.length)} compte(s) éligible(s) à ce run · ${fmtN(vus.length)} affiché(s) après filtres`));
  }

  for (const [el, cle] of [[recherche, "texte"], [selPa, "pa"], [selCtc, "ctc"], [selPpf, "ppf"]]) {
    el.addEventListener(el.tagName === "SELECT" ? "change" : "input", () => {
      filtres[cle] = el.value; dessiner();
    });
  }
  dessiner();

  modal(
    h("h3", { style: "margin:2px 0 0" }, `Ajouter des comptes au run ${run.num}`),
    h("div", { class: "add-run" },
      h("span", {}, "Run ", h("b", {}, run.num), " du ", h("b", {}, fmtDate(run.date))),
      h("span", { class: "jjs" }, "jours de cycle couverts ",
        ...run.jjs.map((j) => h("code", {}, String(j))))),
    h("p", { class: "field-hint", style: "margin-top:-4px" },
      "Seuls les comptes dont le jour de cycle est couvert par ce run sont listés — un run "
      + "ne peut pas facturer un autre jour. Les comptes non prêts sont proposés et signalés : "
      + "les ajouter reste un choix assumé."),
    h("div", { class: "add-filters" }, recherche, selPa, selCtc, selPpf, raz),
    corps,
    h("div", { class: "add-foot" }, pied, h("span", { class: "spacer" }),
      h("button", { class: "btn-ghost", onclick: closeModal }, "Annuler"),
      h("button", { class: "btn-primary", onclick: async () => {
        if (!choisis.size) return;
        try {
          await invoke("plan_ajouter", { cfs: [...choisis], runNum: run.num });
          closeModal(); await rechargerRecap();
        } catch (e) { planBanner("error", String(e)); closeModal(); }
      } }, `Ajouter au run ${run.num}`)));

  // La fenêtre porte un tableau : #modal plafonne à 460px, il lui faut sa
  // variante large. Posée après `modal()`, qui reconstruit l'élément.
  document.getElementById("modal").classList.add("modal-wide");
}
```

⚠️ **Trois points à vérifier sur le code réel avant de coller** : la signature de
`modal(...)` et le nom de `closeModal` (lire l'ancienne `ouvrirAjout` avant de la
supprimer) ; l'existence de `fmtDate` (sinon formater la date comme le fait la
timeline) ; et le fait que `modal()` reconstruise ou non `#modal` — si l'élément
est réutilisé, retirer `modal-wide` à la fermeture pour ne pas élargir les
confirmations suivantes.

- [ ] **Étape 6 : styles de la fenêtre**

Copier dans `client/src/styles.css` le bloc de la maquette délimité par le
commentaire `AJOUTS DU CHANTIER « ajout de comptes par run »`, **sauf** les
règles `#modal-backdrop` (tâche 1), `td.tl-add` / `.tl-add-btn` (tâche 4) et
`.mq-note` (propre à la maquette). Conserver l'indentation des règles voisines.

- [ ] **Étape 7 : vérifier et commit**

```bash
cd client && node --test "tests/*.test.js" 2>&1 | tail -5
git add client/src/app.js client/src/styles.css client/tests/plan_ajout_run.test.js
git commit -m "feat(superpopaul): fenêtre d'ajout triable et filtrable par run"
```

---

# VOLET C — le classeur du périmètre

## Tâche 6 : la dépendance

**Fichiers :**
- Modifier : `client/src-tauri/Cargo.toml`

- [ ] **Étape 1 : ajouter la crate**

Dans `[dependencies]`, à la suite des autres :

```toml
# Classeur XLSX du périmètre du plan : en-tête figé et filtres automatiques,
# qu'un CSV ne peut pas porter. Crate Rust pure, sans dépendance C.
rust_xlsxwriter = "0.93"
```

- [ ] **Étape 2 : vérifier la compilation**

```bash
cd client/src-tauri && cargo build 2>&1 | tail -3
```

Attendu : compilation réussie. Si la version 0.93 n'existe plus, prendre la plus
récente et **le signaler** — ne pas rétrograder silencieusement.

- [ ] **Étape 3 : commit**

```bash
git add client/src-tauri/Cargo.toml client/src-tauri/Cargo.lock
git commit -m "build(superpopaul): dépendance rust_xlsxwriter pour l'export du plan"
```

## Tâche 7 : `plan_xlsx` — composition du tableau (pure)

**Fichiers :**
- Créer : `client/src-tauri/src/plan_xlsx.rs`
- Modifier : `client/src-tauri/src/lib.rs`

- [ ] **Étape 1 : déclarer le module**

Dans `lib.rs`, en respectant l'ordre alphabétique (après `plan_report`) :

```rust
pub mod plan_xlsx;
```

- [ ] **Étape 2 : écrire les tests qui échouent**

Créer `client/src-tauri/src/plan_xlsx.rs` :

```rust
//! Classeur XLSX du périmètre du plan — **tous** les comptes du fichier
//! d'entrée, au plan ou non.
//!
//! La composition du tableau (`lignes`) est PURE et testable ; l'écriture
//! (`ecrire`) n'a aucune logique métier. Même séparation que `charge` et
//! `charts` pour le rapport.

use crate::plan::{LigneEntree, LignePlan};

/// Rapport d'un compte au plan. Trois états, pas deux : un compte **retiré**
/// n'est pas un compte jamais placé — ce sont deux décisions opposées.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appartenance {
    Oui,
    Retire,
    Non,
}

impl Appartenance {
    /// Libellé de la colonne « Dans le plan ».
    pub fn libelle(self) -> &'static str {
        match self {
            Appartenance::Oui => "oui",
            Appartenance::Retire => "retiré",
            Appartenance::Non => "non",
        }
    }
}

/// Une ligne du classeur : un compte du fichier d'entrée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneExport {
    /// Vide si le compte n'a jamais été placé ; **conservé** s'il a été retiré.
    pub run: String,
    pub cf: String,
    /// Le jour de cycle tel qu'il figurait dans le fichier, même illisible.
    pub jj: String,
    /// Adressage sous forme nue quand le schéma s'y prête.
    pub adressage: String,
    pub raison_sociale: String,
    pub ctc_status: String,
    pub ppf_usable: bool,
    pub appartenance: Appartenance,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jour(iso: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    fn entree(cf: &str, jj: &str, ctc: &str) -> LigneEntree {
        LigneEntree {
            cf: cf.into(),
            participant: "iso6523-actorid-upis::0225:12345678900012".into(),
            jj_brut: jj.into(),
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            resolu: true,
            ctc_ready: ctc == "ready",
            ctc_status: ctc.into(),
            ppf_usable: true,
            in_directory: true,
            resolved_at: 0,
        }
    }

    fn ligne_plan(cf: &str, run: &str) -> LignePlan {
        LignePlan {
            cf: cf.into(),
            participant: "0225:12345678900012".into(),
            jj: 5,
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            mep_id: 1,
            mep_date: jour("2026-08-01"),
            run_num: run.into(),
            run_date: jour("2026-08-11"),
            origine: crate::plan::Origine::Auto,
            in_directory: true,
            resolved_at: 0,
            planned_at: 0,
            retire: None,
        }
    }

    #[test]
    fn export_couvre_toutes_les_lignes_du_fichier() {
        // Un compte hors du pool (non résolu, sans plateforme) figure quand même
        // au tableau : « l'intégralité des comptes du fichier ».
        let mut hors_pool = entree("CF2", "5", "");
        hors_pool.resolu = false;
        hors_pool.pa = String::new();
        let entrees = vec![entree("CF1", "5", "ready"), hors_pool];
        let plan = vec![ligne_plan("CF1", "R1")];
        let out = lignes(&entrees, &plan);
        assert_eq!(out.len(), 2);
        let cf2 = out.iter().find(|l| l.cf == "CF2").expect("CF2 absent");
        assert_eq!(cf2.appartenance, Appartenance::Non);
        assert_eq!(cf2.run, "", "jamais placé : pas de run");
    }

    #[test]
    fn un_compte_retire_conserve_son_run_et_vaut_retire() {
        let mut retiree = ligne_plan("CF1", "R1");
        retiree.retire = Some(crate::plan::Retrait { le: 1, motif: "clôturé".into() });
        let out = lignes(&[entree("CF1", "5", "ready")], &[retiree]);
        assert_eq!(out[0].appartenance, Appartenance::Retire);
        assert_eq!(out[0].run, "R1", "le run est la trace de ce dont on l'a sorti");
    }

    #[test]
    fn un_compte_au_plan_porte_son_run() {
        let out = lignes(&[entree("CF1", "5", "ready")], &[ligne_plan("CF1", "R1")]);
        assert_eq!(out[0].appartenance, Appartenance::Oui);
        assert_eq!(out[0].run, "R1");
    }

    #[test]
    fn l_adressage_sort_sous_forme_nue() {
        let out = lignes(&[entree("CF1", "5", "ready")], &[]);
        assert_eq!(out[0].adressage, "0225:12345678900012", "forme canonique non réduite");
    }

    #[test]
    fn un_adressage_non_0225_sort_sous_forme_canonique() {
        // Repli : pas de valeur nue à extraire d'un autre schéma.
        let mut e = entree("CF1", "5", "ready");
        e.participant = "iso6523-actorid-upis::0088:7300010000001".into();
        let out = lignes(&[e], &[]);
        assert_eq!(out[0].adressage, "iso6523-actorid-upis::0088:7300010000001");
    }

    #[test]
    fn le_statut_ctc_nest_pas_aplati() {
        let out = lignes(&[entree("CF1", "5", "later")], &[]);
        assert_eq!(out[0].ctc_status, "later");
    }

    #[test]
    fn le_jour_de_cycle_illisible_est_rendu_tel_quel() {
        // Le classeur documente le fichier, il ne le corrige pas.
        let out = lignes(&[entree("CF1", "zzz", "")], &[]);
        assert_eq!(out[0].jj, "zzz");
    }
}
```

- [ ] **Étape 3 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client/src-tauri && cargo test plan_xlsx::
```

Attendu : `cannot find function 'lignes' in this scope`.

- [ ] **Étape 4 : implémenter la composition**

Ajouter avant `#[cfg(test)]` :

```rust
/// Compose le tableau : une ligne par compte du fichier d'entrée, dans l'ordre
/// où il les fournit. PURE — ni disque, ni format.
pub fn lignes(entrees: &[LigneEntree], plan: &[LignePlan]) -> Vec<LigneExport> {
    let par_cf: std::collections::HashMap<&str, &LignePlan> =
        plan.iter().map(|l| (l.cf.as_str(), l)).collect();

    entrees
        .iter()
        .map(|e| {
            let (run, appartenance) = match par_cf.get(e.cf.as_str()) {
                Some(l) if l.retiree() => (l.run_num.clone(), Appartenance::Retire),
                Some(l) => (l.run_num.clone(), Appartenance::Oui),
                None => (String::new(), Appartenance::Non),
            };
            LigneExport {
                run,
                cf: e.cf.clone(),
                jj: e.jj_brut.clone(),
                adressage: crate::directory::parse_0225_value(&e.participant)
                    .unwrap_or_else(|| e.participant.clone()),
                raison_sociale: e.raison_sociale.clone(),
                ctc_status: e.ctc_status.clone(),
                ppf_usable: e.ppf_usable,
                appartenance,
            }
        })
        .collect()
}
```

⚠️ **Vérifier `LignePlan::retiree()` et `directory::parse_0225_value`** sur le
code réel avant de vous en servir : `retiree()` est utilisée par `plan_report`,
`parse_0225_value` par `commands.rs`. Si `parse_0225_value` ne rend pas
`Option<String>`, adapter — sans modifier la fonction.

- [ ] **Étape 5 : lancer les tests et commit**

```bash
cd client/src-tauri && cargo test plan_xlsx::
git add client/src-tauri/src/plan_xlsx.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): composition du tableau de périmètre du plan"
```

Attendu : 7 tests `ok`.

## Tâche 8 : `plan_xlsx::ecrire` et branchement

**Fichiers :**
- Modifier : `client/src-tauri/src/plan_xlsx.rs`
- Modifier : `client/src-tauri/src/commands.rs` (`plan_generate`, vers 1009)

- [ ] **Étape 1 : écrire l'écriture du classeur**

Ajouter dans `plan_xlsx.rs` :

```rust
use std::path::Path;

/// En-têtes du classeur, dans l'ordre des colonnes.
const ENTETES: [&str; 8] = [
    "N° de run",
    "N° de CF",
    "JJ",
    "Adressage",
    "Raison sociale",
    "Statut CTC",
    "PPF usable",
    "Dans le plan",
];

/// Écrit le classeur : en-tête figé et filtres automatiques, qui sont l'usage
/// attendu du fichier. Aucune logique métier ici.
pub fn ecrire(chemin: &Path, lignes: &[LigneExport]) -> Result<(), String> {
    use rust_xlsxwriter::{Format, Workbook};

    let mut classeur = Workbook::new();
    let feuille = classeur.add_worksheet();
    feuille
        .set_name("Comptes")
        .map_err(|e| format!("classeur : {e}"))?;

    let gras = Format::new().set_bold();
    for (i, titre) in ENTETES.iter().enumerate() {
        feuille
            .write_with_format(0, i as u16, *titre, &gras)
            .map_err(|e| format!("en-tête : {e}"))?;
    }

    for (n, l) in lignes.iter().enumerate() {
        let r = n as u32 + 1;
        let cellules: [&str; 8] = [
            &l.run,
            &l.cf,
            &l.jj,
            &l.adressage,
            &l.raison_sociale,
            &l.ctc_status,
            if l.ppf_usable { "true" } else { "false" },
            l.appartenance.libelle(),
        ];
        for (c, v) in cellules.iter().enumerate() {
            feuille
                .write(r, c as u16, *v)
                .map_err(|e| format!("ligne {r} : {e}"))?;
        }
    }

    // En-tête figé et filtres : sans eux, le fichier n'est qu'un CSV déguisé.
    feuille.set_freeze_panes(1, 0).map_err(|e| format!("volets : {e}"))?;
    feuille
        .autofilter(0, 0, lignes.len() as u32, ENTETES.len() as u16 - 1)
        .map_err(|e| format!("filtres : {e}"))?;
    for (i, largeur) in [12.0, 16.0, 6.0, 30.0, 38.0, 12.0, 12.0, 14.0].iter().enumerate() {
        feuille
            .set_column_width(i as u16, *largeur)
            .map_err(|e| format!("largeur : {e}"))?;
    }

    classeur
        .save(chemin)
        .map_err(|e| format!("écriture du classeur : {e}"))
}
```

**Toutes les valeurs sont écrites en texte**, y compris le JJ : le fichier
documente ce que contenait le CSV, y compris un jour de cycle illisible, et
Excel ne doit pas réinterpréter un identifiant en nombre.

- [ ] **Étape 2 : écrire le test d'écriture**

Ajouter dans `mod tests` :

```rust
#[test]
fn ecrire_produit_un_fichier_lisible() {
    // On ne relit pas le xlsx (pas de lecteur dans les dépendances) : on
    // vérifie qu'un fichier non vide est produit et qu'il porte la signature
    // d'un conteneur ZIP, ce qu'est un .xlsx.
    let dir = std::env::temp_dir().join("popaul_test_xlsx");
    std::fs::create_dir_all(&dir).unwrap();
    let chemin = dir.join("t.xlsx");
    let l = lignes(&[entree("CF1", "5", "ready")], &[ligne_plan("CF1", "R1")]);
    ecrire(&chemin, &l).expect("écriture");
    let octets = std::fs::read(&chemin).expect("relecture");
    assert!(octets.len() > 1000, "classeur suspect : {} octets", octets.len());
    assert_eq!(&octets[..2], b"PK", "un .xlsx est un conteneur ZIP");
    std::fs::remove_file(&chemin).ok();
}

#[test]
fn ecrire_un_tableau_vide_ne_panique_pas() {
    let dir = std::env::temp_dir().join("popaul_test_xlsx");
    std::fs::create_dir_all(&dir).unwrap();
    let chemin = dir.join("vide.xlsx");
    ecrire(&chemin, &[]).expect("un plan sans compte reste un fichier valide");
    std::fs::remove_file(&chemin).ok();
}
```

- [ ] **Étape 3 : lancer les tests**

```bash
cd client/src-tauri && cargo test plan_xlsx::
```

Attendu : 9 tests `ok`.

- [ ] **Étape 4 : brancher sur « Générer le plan »**

Dans `commands.rs`, commande `plan_generate` (vers 1009), après la boucle qui
écrit les `<souche>_plan_mep_<n>_<date>.txt` (vers 1090), ajouter :

```rust
        // Le classeur du périmètre part avec les fichiers de livraison : ce
        // qu'on transmet et ce qui le documente restent ainsi cohérents.
        let xlsx = dir.join(format!("{souche}_plan_comptes.xlsx"));
        crate::plan_xlsx::ecrire(&xlsx, &crate::plan_xlsx::lignes(&entrees, &lignes))?;
```

⚠️ **Les noms `dir`, `souche`, `entrees` et `lignes` sont ceux supposés du
corps de `plan_generate`.** Lire la fonction et utiliser ses variables réelles :
si les entrées ne sont pas disponibles à ce point, les obtenir comme le fait
`plan_rapport` (`commands.rs:1444`, `plan_entrees_from_scan`). Signaler l'écart.

- [ ] **Étape 5 : vérifier et commit**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -3 && cargo clippy --lib --tests 2>&1 | grep -A1 "^warning: " | grep -c "^\s*--> "
git add client/src-tauri/src/
git commit -m "feat(superpopaul): classeur du périmètre produit avec le plan"
```

Attendu : suite verte, **5** warnings clippy.

---

# VÉRIFICATION

## Tâche 9 : passe de mutation

**Les mutations se dérivent du code écrit, jamais de ce plan.** Au lot
précédent, celles écrites d'avance visaient ce qu'on croyait avoir testé, et un
audit lisant le code en a trouvé huit qui survivaient toutes.

- [ ] **Étape 1 : muter le filtrage par run**

Pour chaque mutation : appliquer, lancer `cargo test`, constater, **restaurer**.

| # | Mutation | Test qui doit tomber |
|---|----------|----------------------|
| 1 | `run.couvre(jj)` → `true` | `candidats_run_ne_rend_que_les_jours_de_cycle_couverts` |
| 2 | filtre `!deja_au_plan.contains` supprimé | `candidats_run_exclut_les_comptes_deja_au_plan` |
| 3 | `ctc_status: e.ctc_status.clone()` → `String::new()` | `candidats_run_porte_le_statut_ctc_complet` |
| 4 | `eligible: e.ctc_ready && e.ppf_usable` → `true` | `candidats_run_rend_les_non_eligibles_signales` |

- [ ] **Étape 2 : muter la composition du classeur**

| # | Mutation | Test qui doit tomber |
|---|----------|----------------------|
| 5 | `Some(l) if l.retiree()` → branche supprimée | `un_compte_retire_conserve_son_run_et_vaut_retire` |
| 6 | `run` vidé pour les retirés | `un_compte_retire_conserve_son_run_et_vaut_retire` |
| 7 | `parse_0225_value(...)` → `e.participant.clone()` | `l_adressage_sort_sous_forme_nue` |
| 8 | `set_freeze_panes` et `autofilter` supprimés | **aucun** — trou connu, cf. étape 4 |

- [ ] **Étape 3 : muter les fonctions JS**

| # | Mutation | Test qui doit tomber |
|---|----------|----------------------|
| 9 | `filtrerCandidats` : `if (f.pa && …)` supprimé | `les filtres se combinent` |
| 10 | `trierCandidats` : `croissant ? d : -d` → `d` | `le tri par colonne réordonne la liste` |
| 11 | recherche : `.toLowerCase()` retiré des deux côtés | `la recherche porte sur le compte…` |

- [ ] **Étape 4 : rendre compte des trous**

La mutation 8 ne peut pas être détectée sans lecteur XLSX dans les dépendances —
le test d'écriture ne vérifie que la signature ZIP et la taille. **Le dire dans
le compte rendu** plutôt que de laisser croire à une couverture complète : la
présence des filtres et du volet figé se vérifie au parcours GUI, pas en test.

Si une autre mutation survit, le test correspondant est creux : le corriger et
le signaler.

- [ ] **Étape 5 : vérifier le retour à l'état initial**

```bash
git status --short && cd client/src-tauri && cargo test 2>&1 | tail -3
```

Attendu : aucun diff, suite verte.

## Tâche 10 : parcours GUI

Aucun test ne remplace cette étape : les défauts de superposition, de tri à
l'écran et d'ouverture du classeur ne se voient pas autrement.

- [ ] **Étape 1 : lancer l'application**

```bash
cd client && npm run tauri dev
```

- [ ] **Étape 2 : vérifier le volet A**

Ouvrir le Plan de charge, déclencher une modale (par exemple « Retirer… » depuis
le récap) : elle doit s'afficher **au-dessus** de l'écran du plan.

- [ ] **Étape 3 : vérifier le volet B**

Sur la timeline : les runs retenus portent « + Ajouter », les runs écartés non.
Ouvrir la fenêtre depuis un run et vérifier : le bandeau nomme le bon run et ses
jours de cycle ; la liste ne contient que des comptes de ces jours ; le tri
fonctionne sur chaque colonne ; les filtres se combinent ; « réinitialiser »
restaure la liste ; un compte non éligible est marqué ⚠ et reste sélectionnable ;
l'ajout place bien les comptes sur ce run.

Vérifier aussi que le bouton global a disparu de l'onglet *Comptes de
facturation* et que « Déplacer vers un run… » fonctionne toujours.

- [ ] **Étape 4 : vérifier le volet C**

Générer le plan, ouvrir `<souche>_plan_comptes.xlsx` dans Excel : en-tête figé au
défilement, filtres automatiques présents sur les huit colonnes, comptes au plan
avec leur run, comptes retirés marqués « retiré » **avec** leur run, comptes
absents du plan avec un run vide et « non », adressages en `0225:…`.

- [ ] **Étape 5 : rendre compte**

Signaler tout écart avec la maquette validée. Ne pas déclarer le lot terminé
avant que ce parcours soit passé.

---

## Définition de terminé

- [ ] `cargo test` vert — **497 + 15 neufs = 512** attendus, répartis en
      1 (`plan.rs`, tâche 2), 5 (`commands.rs`, tâche 3), 7 (`plan_xlsx.rs`,
      tâche 7) et 2 (écriture, tâche 8). Donner le chiffre réel et expliquer
      tout écart.
- [ ] `node --test "tests/*.test.js"` vert — 29 + 4 neufs = **33**
- [ ] `cargo clippy --lib --tests` : 5 warnings, les préexistants, aucun neuf
- [ ] Passe de mutation faite, trous signalés explicitement
- [ ] Parcours GUI validé par l'utilisateur
- [ ] `git status` propre, commits en `feat(superpopaul): …` / `fix(…)` / `build(…)`
