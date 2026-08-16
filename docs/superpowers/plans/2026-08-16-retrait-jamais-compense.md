# Un retrait n'est jamais compensé — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** Un retrait ne libère plus sa part de cible (aucun re-tirage de
compensation), « Exclure le run » s'offre aussi pour un run à venir d'une MEP
gelée, l'aperçu se recalcule et le rapport HTML du plan se réécrit après chaque
retouche.

**Architecture :** Moteur pur d'abord (`plan.rs`, TDD), puis livrable
(`commands.rs` : rapport HTML factorisé et appelé partout où le plan s'écrit),
puis IHM (`app.js` : 3ᵉ segment de la modale — maquette validée avant —, et
épilogue commun des retouches). Spec : `docs/superpowers/specs/2026-08-16-retrait-jamais-compense-design.md`.

**Tech stack :** Rust (cargo test dans `client/src-tauri/`), JS vanilla testé
par faux DOM (`node --test "tests/*.test.js"` depuis `client/`).

**Règles projet à respecter :** commits `feat(superpopaul):` /
`fix(superpopaul):` / `docs(superpopaul):` ; texte UI en français ; jamais
d'innerHTML dynamique (helper `h()`) ; la maquette (tâche 3) exige un **go
explicite de l'utilisateur** avant la tâche 4 — c'est un point d'arrêt, pas une
formalité.

---

## Tâche 1 : Moteur — les retirées consomment leur part (`plan.rs`)

**Files:**
- Modify: `client/src-tauri/src/plan.rs` (`Preserves::consomme` ~l.1290,
  `cible_auto` ~l.1298, `allouer` ~l.634-646, tests ~l.2417-2470)

- [ ] **Step 1 : inverser les deux tests qui encodent l'ancienne règle**

Dans `plan.rs`, test `preserves_le_retrait_prime_sur_le_gel` (~l.2418),
remplacer la dernière assertion :

```rust
        assert_eq!(p.consomme(), 1, "un retrait n'est jamais compensé : la place reste occupée");
```

Renommer le test `cible_auto_ignore_les_retirees` (~l.2461) et inverser son
attendu :

```rust
    #[test]
    fn cible_auto_compte_une_retiree_hors_pool() {
        // Décision du 16/08/2026 (inverse celle du 28/07) : un retrait est une
        // place qu'on a décidé de ne pas livrer, mais qui reste OCCUPÉE — sans
        // elle dans la cible, la régénération tirerait un remplaçant.
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let mut retiree = lp("HORS", 5, "PA", "2026-12-01", Origine::Manuel);
        retiree.retire = Some(Retrait { le: 0, motif: "clôturé".into() });
        let p = Preserves { retirees: vec![retiree], ..Preserves::default() };
        assert_eq!(cible_auto(&pool, &p), 6);
    }
```

- [ ] **Step 2 : ajouter les deux tests d'invariant « aucun remplaçant »**

Dans le même `mod tests`, à la suite des tests `regeneration_*` :

```rust
    #[test]
    fn le_retrait_d_une_gelee_n_est_pas_compense() {
        // Le vécu du 16/08 : alléger une MEP gelée, recalculer l'aperçu — la
        // rampe re-tirait des remplaçants pour tenir la cible. Un retrait
        // réduit le volume livré, définitivement.
        let pool: Vec<CfCandidat> = (0..6).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let rs = runs_jj(2, &[5]);
        let gelee = lp("GEL", 5, "PA", "2026-01-01", Origine::Auto);
        let mut retiree = lp("RET", 5, "PA", "2026-01-01", Origine::Auto);
        retiree.retire = Some(Retrait { le: 1, motif: "allégé".into() });
        let p = Preserves { gelees: vec![gelee], retirees: vec![retiree], ..Preserves::default() };

        // Cible 8 = 6 du pool + la gelée + la retirée (toutes deux hors pool).
        let a = regenerer(&pool, &rs, &meps1(), 42, 8, &rampe(Forme::Plate), &p).unwrap();
        assert!(a.avertissements.is_empty(), "aucun compte ne manque : {:?}", a.avertissements);
        let actives = a.lignes.iter().filter(|l| !l.retiree()).count();
        assert_eq!(actives, 7, "la place de RET reste vide, personne ne la prend");
    }

    #[test]
    fn le_retrait_d_une_auto_ne_deplace_pas_les_quotas_de_plateforme() {
        // Deux plateformes, cible partielle : si la place retirée était rendue
        // au tirage, sa plateforme recevrait un REMPLAÇANT — c'est exactement
        // ce que le retrait interdit. Les actifs d'après doivent être ceux
        // d'avant, moins la retirée, à l'identique.
        let mut pool: Vec<CfCandidat> = (0..4).map(|i| cand(&format!("A{i}"), 5, "Esker")).collect();
        pool.extend((0..4).map(|i| cand(&format!("B{i}"), 5, "Cegedim")));
        let rs = runs_jj(2, &[5]);
        let avant = regenerer(&pool, &rs, &meps1(), 42, 6, &rampe(Forme::Plate), &Preserves::default())
            .unwrap()
            .lignes;
        let mut retiree = avant.iter().find(|l| l.pa == "Esker").expect("Esker est servie").clone();
        retiree.retire = Some(Retrait { le: 1, motif: "allégé".into() });
        let p = Preserves { retirees: vec![retiree.clone()], ..Preserves::default() };

        let apres = regenerer(&pool, &rs, &meps1(), 42, 6, &rampe(Forme::Plate), &p).unwrap();
        assert!(apres.avertissements.is_empty(), "{:?}", apres.avertissements);
        let actifs: HashSet<&str> =
            apres.lignes.iter().filter(|l| !l.retiree()).map(|l| l.cf.as_str()).collect();
        let attendus: HashSet<&str> = avant
            .iter()
            .map(|l| l.cf.as_str())
            .filter(|c| *c != retiree.cf.as_str())
            .collect();
        assert_eq!(actifs, attendus, "les actifs restants sont ceux d'avant, sans remplaçant");
    }
```

- [ ] **Step 3 : vérifier que les quatre tests échouent**

Run : `cd client/src-tauri && cargo test consomme retrait_d cible_auto_compte 2>&1 | tail -20`
Attendu : ÉCHEC des 4 (assertions `consomme() == 1`, `cible_auto == 6`,
avertissement « cible non atteinte » présent, remplaçant tiré).

- [ ] **Step 4 : implémenter**

`Preserves::consomme()` (~l.1290) — remplacer corps et commentaire :

```rust
    /// Part de cible déjà consommée : gelées, épinglées ET retirées. Un
    /// retrait n'est jamais compensé (décision du 16/08/2026, qui inverse
    /// celle du 28/07) : la place d'un compte retiré reste la sienne, la
    /// rampe ne tire pas de remplaçant.
    pub fn consomme(&self) -> usize {
        self.gelees.len() + self.epinglees.len() + self.retirees.len()
    }
```

`cible_auto` (~l.1307) — inclure les retirées dans `hors_pool` (et retoucher le
doc-commentaire ~l.1298-1306 dans le même esprit : « préservées » couvre
désormais les retirées) :

```rust
    let hors_pool = preserves
        .gelees
        .iter()
        .chain(&preserves.epinglees)
        .chain(&preserves.retirees)
        .filter(|l| !au_pool.contains(l.cf.as_str()))
        .count();
```

`allouer` (~l.634-646) — les retirées occupent aussi leur place dans les
quotas ; les DEUX boucles de semence gagnent `.chain(&preserves.retirees)` :

```rust
    // Quotas sur le plan COMPLET (préservées incluses, retirées comprises) :
    // sinon une plateforme déjà servie par le gel — ou délestée par un
    // retrait — recevrait encore une part pleine.
    let mut stock_par_pa: BTreeMap<String, usize> =
        par_pa.iter().map(|(h, v)| (h.clone(), v.len())).collect();
    let mut places_par_pa: HashMap<&str, usize> = HashMap::new();
    for l in preserves.gelees.iter().chain(&preserves.epinglees).chain(&preserves.retirees) {
        *stock_par_pa.entry(l.pa.clone()).or_insert(0) += 1;
    }
    let quotas = quotas_par_pa(cible + preserves.consomme(), &stock_par_pa);
    for l in preserves.gelees.iter().chain(&preserves.epinglees).chain(&preserves.retirees) {
        *places_par_pa.entry(l.pa.as_str()).or_insert(0) += 1;
    }
```

- [ ] **Step 5 : suite Rust complète**

Run : `cd client/src-tauri && cargo test 2>&1 | tail -5`
Attendu : PASS. Si un test existant casse, ne pas l'absorber : vérifier s'il
encode l'ancienne règle (alors l'inverser avec un commentaire datant la
décision) ou révèle un vrai défaut (alors s'arrêter et le signaler).

- [ ] **Step 6 : commit**

```bash
git add client/src-tauri/src/plan.rs
git commit -m "feat(superpopaul): un retrait n'est jamais compensé par un re-tirage"
```

- [ ] **Step 7 : recenser les traces restantes de l'ancienne règle**

Run : `grep -rn "ne consomme pas\|ne comptent pas\|retirées ne\|replacera d'autres" client/src-tauri/src docs/superpowers/specs`
Attendu : plus aucune occurrence qui affirme l'ancienne règle dans `src/`
(commentaires compris) ; corriger ce qui reste, en datant la décision.

- [ ] **Step 8 : lever la limite documentée du 14/08**

Dans `docs/superpowers/specs/2026-08-14-alleger-un-run-design.md`, § 5, à la
fin du premier point (« …exclus du re-tirage, comportement existant). »),
ajouter :

```
  **Levée le 16/08/2026** : un retrait n'est plus compensé — voir
  `2026-08-16-retrait-jamais-compense-design.md`.
```

```bash
git add docs/superpowers/specs/2026-08-14-alleger-un-run-design.md
git commit -m "docs(superpopaul): la limite « la régénération replace le volume retiré » est levée"
```

---

## Tâche 2 : Le rapport HTML du plan part avec les livrables (`commands.rs`)

**Files:**
- Modify: `client/src-tauri/src/commands.rs` (`plan_rapport` ~l.2189,
  `sauver_apres_retouche` ~l.1622, `plan_generate` ~l.1198-1205, tests du
  mod `tests_rapprochement` ~l.3178)

- [ ] **Step 1 : écrire le test défaillant**

Dans `mod tests_rapprochement` (il possède déjà `params_avec_run_exclu()` et
`meta_pour()`), ajouter — en reprenant la construction de `Config` et de
`LignePlan` du test `avertissement_annuaire_ne_rejoint_plus_les_avertissements_du_calcul`
(~l.3286-3341) :

```rust
    #[test]
    fn une_retouche_reecrit_le_rapport_html_du_plan() {
        // Le vécu du 16/08 : après une exclusion, `<souche>_plan.html`
        // décrivait encore le plan d'avant. Le rapport est un livrable : il
        // part avec les fichiers de MEP et le classeur, à chaque écriture.
        let store = Arc::new(Mutex::new(Store::open_in_memory().expect("store en mémoire")));
        let params = params_avec_run_exclu();
        let meta = meta_pour(&params);
        let lignes = vec![/* copier le LignePlan CF1 du test annuaire (~l.3296) */];

        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("brm.csv");
        std::fs::write(&csv, "cf;pid;jj\nCF1;;5\n").expect("écriture du CSV de test");
        let cfg = /* copier la Config du test annuaire (~l.3319), avec
                     output.dir = dir.path().to_string_lossy().into_owned() */;

        sauver_apres_retouche(&store, &csv, &cfg, &lignes, &meta).expect("retouche sauvée");

        let rapport = dir.path().join("brm_plan.html");
        assert!(rapport.exists(), "le rapport se réécrit avec les autres livrables");
        let html = std::fs::read_to_string(&rapport).unwrap();
        assert!(html.contains("CF1"), "le rapport décrit le plan courant");
    }
```

Avant d'écrire l'assertion de contenu, vérifier dans `plan_report.rs` (et ses
tests) ce que le rendu affiche d'une ligne du plan ; si le n° de CF n'y figure
pas tel quel, asserter sur un marqueur stable du rendu (leçon « une chaîne
cherchée a plusieurs producteurs » : choisir un marqueur à producteur unique).

- [ ] **Step 2 : vérifier l'échec**

Run : `cd client/src-tauri && cargo test une_retouche_reecrit 2>&1 | tail -10`
Attendu : ÉCHEC — `brm_plan.html` n'existe pas.

- [ ] **Step 3 : factoriser `ecrire_rapport_plan` et le brancher**

Sous `sauver_apres_retouche`, extraire du corps de `plan_rapport`
(~l.2194-2230) :

```rust
/// Écrit le rapport HTML du plan. Livrable au même titre que les fichiers de
/// MEP et le classeur (décision du 16/08/2026) : chaque écriture du plan le
/// régénère — un rapport qui décrit le plan d'avant est pire que pas de
/// rapport. `entrees` vient de l'appelant : le scan est déjà fait pour le
/// classeur, le refaire ici doublerait la lecture du fichier.
fn ecrire_rapport_plan(
    input: &Path,
    dir: &str,
    lignes: &[crate::plan::LignePlan],
    meta: &crate::store::PlanMeta,
    entrees: &[crate::plan::LigneEntree],
) -> Result<PathBuf, String> {
    let params = crate::plan::PlanParams::depuis_yaml(&meta.params_yaml)?;
    let (pool, _) = crate::plan::construire_pool(entrees, &params.pa_exclues())?;
    let mut pool_par_pa: std::collections::BTreeMap<String, usize> = Default::default();
    for c in &pool {
        *pool_par_pa.entry(c.pa.clone()).or_insert(0) += 1;
    }
    let mut pool_par_jj: std::collections::BTreeMap<u8, usize> = Default::default();
    for c in &pool {
        *pool_par_jj.entry(c.jj).or_insert(0) += 1;
    }
    let (runs, _meps) = calendrier_du_plan(meta)?;
    let maintenant = chrono::Local::now();
    let html = crate::plan_report::render(&crate::plan_report::PlanReportData {
        fichier: &meta.fichier,
        date_longue: &report::date_fr_longue(&maintenant),
        version: env!("CARGO_PKG_VERSION"),
        lignes,
        aujourdhui: maintenant.date_naive(),
        pool_par_pa: &pool_par_pa,
        pool_par_jj: &pool_par_jj,
        runs: &runs,
    });
    let out = resolved_out_dir(input, dir).join(format!(
        "{}_plan.html",
        input.file_stem().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&out, html).map_err(|e| format!("écriture du rapport de plan : {e}"))?;
    Ok(out)
}
```

(Adapter les détails d'emprunt — noms de champs de `PlanReportData`, signature
exacte — au corps réel de `plan_rapport`, qui reste la référence.)

Brancher trois appelants :

1. `plan_rapport` : son corps devient `charger_pour_retouche` + scan des
   entrées + `ecrire_rapport_plan(...)` puis `Ok(out.display().to_string())`.
2. `sauver_apres_retouche` : après l'écriture du classeur, ajouter
   `ecrire_rapport_plan(input, &cfg.output.dir, lignes, meta, &entrees)?;` et
   étendre son doc-commentaire (« Réécrit plan ET fichiers. Les trois vont
   ensemble » → les QUATRE : plan, fichiers de MEP, classeur, rapport).
3. `plan_generate` : après l'écriture du classeur (~l.1201-1204), ajouter
   `ecrire_rapport_plan(&input, &cfg.output.dir, &lignes, &meta, &entrees)?;`.

- [ ] **Step 4 : vérifier le passage et la suite complète**

Run : `cd client/src-tauri && cargo test 2>&1 | tail -5`
Attendu : PASS, y compris les tests `fichiers_obsoletes` (~l.2550, 2813) qui
vérifient déjà que le ménage des MEP épargne `*_plan.html`.

- [ ] **Step 5 : commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): le rapport HTML du plan se réécrit à chaque génération et retouche"
```

---

## Tâche 3 : Maquette de la modale — POINT D'ARRÊT go utilisateur

**Files:**
- Create: `docs/superpowers/maquettes/2026-08-16-alleger-run-gele-a-venir.html`

- [ ] **Step 1 : dériver la maquette de l'existante**

Copier `docs/superpowers/maquettes/2026-08-14-alleger-un-run.html` (palette
« Bleu nuit & or » déjà en place) et la faire montrer l'état nouveau : modale
« Alléger le run RF01 » d'un **run à venir dont la MEP est gelée**, avec :
- la bascule à **trois segments** : « Retirer N — répartition conservée » /
  « Ne garder que ma sélection » / « Exclure le run » ;
- le segment « Exclure le run » actif : corps de confirmation (mêmes textes que
  le mode existant du run passé), motif pré-rempli
  « Run RF01 du 20/09/2026 exclu — » (SANS « a posteriori »), bouton de retrait
  inerte ;
- l'avertissement « ⚠ N compte(s) appartiennent à une MEP gelée… » visible ;
- en regard, l'état inchangé d'un run à venir non gelé (deux segments), pour
  que la différence se voie.

- [ ] **Step 2 : servir et présenter**

Run : `/Users/jean-paulgavini/.claude/scripts/serve.sh docs/superpowers/maquettes`
(jamais `python3 -m http.server`). Donner l'URL à l'utilisateur.

- [ ] **Step 3 : ATTENDRE LE GO EXPLICITE**

Ne pas entamer la tâche 4 sans un go de l'utilisateur sur cette maquette.
Amender la maquette jusqu'au go si besoin.

- [ ] **Step 4 : commit**

```bash
git add docs/superpowers/maquettes/2026-08-16-alleger-run-gele-a-venir.html
git commit -m "docs(superpopaul): maquette — exclure un run à venir d'une MEP gelée"
```

---

## Tâche 4 : Modale — « Exclure le run » pour un run à venir gelé (`app.js`)

**Files:**
- Modify: `client/src/app.js` (`ouvrirAllegerRun` ~l.3051-3311)
- Test: `client/tests/alleger.test.js`

- [ ] **Step 1 : écrire les tests défaillants**

Dans `alleger.test.js`, en réutilisant `ecran`, `ligne`, `bouton`,
`taperMotif`, `exclusions`, `JOUR_FUTUR`, `RUN` et la manière dont les tests
existants ouvrent la modale (`ctx.app.ouvrirAllegerRun(...)`) :

```js
/** Les libellés des segments de la bascule de la modale ouverte. */
function segments($) {
  const barre = trouver($("modal"), (n) => n.className === "modes");
  return (barre?.children ?? []).map((b) => b.textContent);
}

/** Clique un bouton comme l'utilisateur : par son écouteur. */
const cliquer = (b) => b.listeners.onclick({ currentTarget: b });

test("un run à venir d'une MEP gelée offre aussi l'exclusion", async () => {
  // Ses lignes étant préservées telles quelles, ni la case « exclure » du
  // calcul ni une régénération ne les retireront : l'exclusion a posteriori
  // est le SEUL levier pour vider un tel run (décision du 16/08).
  const ctx = ecran([ligne("CF1", { gelee: true }), ligne("CF2", { gelee: true })]);
  await ctx.app.ouvrirAllegerRun(ctx.evaluer(`(${JSON.stringify(RUN)})`),
    ctx.evaluer(`(${JSON.stringify(JOUR_FUTUR)})`));
  assert.deepEqual(segments(ctx.$), [
    "Retirer N — répartition conservée",
    "Ne garder que ma sélection",
    "Exclure le run",
  ]);
});

test("un run à venir d'une MEP future n'offre pas l'exclusion", async () => {
  const ctx = ecran([ligne("CF1")]); // gelee: false par défaut
  await ctx.app.ouvrirAllegerRun(ctx.evaluer(`(${JSON.stringify(RUN)})`),
    ctx.evaluer(`(${JSON.stringify(JOUR_FUTUR)})`));
  assert.deepEqual(segments(ctx.$), [
    "Retirer N — répartition conservée",
    "Ne garder que ma sélection",
  ]);
});

test("l'exclusion d'un run à venir gelé pré-remplit sans « a posteriori » et exige la cause", async () => {
  const ctx = ecran([ligne("CF1", { gelee: true }), ligne("CF2", { gelee: true })]);
  await ctx.app.ouvrirAllegerRun(ctx.evaluer(`(${JSON.stringify(RUN)})`),
    ctx.evaluer(`(${JSON.stringify(JOUR_FUTUR)})`));
  cliquer(bouton(ctx.$, "Exclure le run"));

  const PREF = "Run RF01 du 05/01/2099 exclu — ";
  assert.equal(champMotif(ctx.$).value, PREF, "pré-rempli, sans « a posteriori »");
  // « Retirer 2 » et pas « Retirer » : le préfixe nu attraperait le SEGMENT
  // « Retirer N — répartition conservée » avant le bouton de validation.
  assert.equal(bouton(ctx.$, "Retirer 2").disabled, true,
    "le pré-rempli ne suffit pas : il faut une cause");

  taperMotif(ctx.$, `${PREF}comptes migrés chez le client`);
  const retirer = bouton(ctx.$, "Retirer 2");
  assert.notEqual(retirer.disabled, true);
  await retirer.listeners.onclick({ currentTarget: retirer });
  assert.equal(exclusions(ctx).length, 1, "un run exclu = UN appel, liste établie côté moteur");
});

test("quitter le segment exclusion sans avoir écrit efface le pré-rempli", async () => {
  const ctx = ecran([ligne("CF1", { gelee: true })]);
  await ctx.app.ouvrirAllegerRun(ctx.evaluer(`(${JSON.stringify(RUN)})`),
    ctx.evaluer(`(${JSON.stringify(JOUR_FUTUR)})`));
  cliquer(bouton(ctx.$, "Exclure le run"));
  cliquer(bouton(ctx.$, "Retirer N"));
  assert.equal(champMotif(ctx.$).value, "",
    "le pré-rempli appartient au mode exclusion, pas aux autres motifs");
});
```

Adapter la mécanique de clic (`cliquer`/`listeners.onclick`) à ce que le
fichier fait déjà pour les segments existants — le mimer, pas l'inventer.

- [ ] **Step 2 : vérifier l'échec**

Run : `cd client && node --test tests/alleger.test.js 2>&1 | tail -15`
Attendu : ÉCHEC des 4 nouveaux tests (2 segments seulement, pas de pré-rempli).

- [ ] **Step 3 : implémenter dans `ouvrirAllegerRun`**

Après le calcul de `actifs` (~l.3060) :

```js
  // Toutes les lignes d'un run partagent sa MEP : « gelé » se lit sur
  // n'importe laquelle — `every` par principe, un run mixte serait un bug.
  const gele = actifs.every((l) => l.gelee);
```

Le pré-rempli (~l.3068) devient commun aux deux cas d'exclusion :

```js
  // « a posteriori » n'a de sens que pour un run déjà joué ; un run à venir
  // gelé s'exclut avant d'avoir joué, son motif le dit sans ce mot.
  const PREREMPLI = `Run ${run.num} du ${fmtDateFr(jour.date)} exclu${passe ? " a posteriori" : ""} — `;
```

`majBascule` (~l.3277) gagne le troisième segment et la vie du pré-rempli :

```js
  function majBascule() {
    if (passe) return;
    const segments = [["prorata", "Retirer N — répartition conservée"],
                      ["selection", "Ne garder que ma sélection"]];
    // Un run à venir d'une MEP gelée ne peut se vider QUE par l'exclusion :
    // ses lignes préservées survivraient à la case « exclure » du calcul
    // comme à une régénération (décision du 16/08).
    if (gele) segments.push(["exclure", "Exclure le run"]);
    bascule.replaceChildren(...segments
      .map(([cle, libelle]) => h("button", { class: mode === cle ? "on" : "", onclick: () => {
        if (mode === cle) return;
        // Le pré-rempli appartient au mode exclusion : posé en y entrant si
        // rien n'est écrit, repris en le quittant s'il est resté tel quel.
        if (cle === "exclure" && zone.value.trim() === "") zone.value = PREREMPLI;
        if (mode === "exclure" && zone.value === PREREMPLI) zone.value = "";
        mode = cle;
        majBascule();
        dessinerCorps();
      } }, libelle)));
  }
```

Rien d'autre ne bouge : `corpsExclure`, `motifSuffisant`, `rafraichir`,
l'avertissement MEP gelée et l'appel `plan_exclure_run` servent déjà les deux
cas ; `passe` continue de piloter le mode unique du run joué, l'affichage des
jours de cycle et `modal-wide`.

- [ ] **Step 4 : vérifier le passage**

Run : `cd client && node --test tests/alleger.test.js 2>&1 | tail -5`
Attendu : PASS, anciens tests compris (le run passé garde son mode unique).

- [ ] **Step 5 : commit**

```bash
git add client/src/app.js client/tests/alleger.test.js
git commit -m "feat(superpopaul): la modale offre l'exclusion d'un run à venir d'une MEP gelée"
```

---

## Tâche 5 : Câblage — récap ET aperçu après chaque retouche (`app.js`)

**Files:**
- Modify: `client/src/app.js` (7 sites : ~l.2417 déplacement, ~l.2451 retrait
  récap, ~l.2506 réactivation, ~l.2728 et ~l.2799 rapprochement appliqué,
  ~l.2991 ajout, ~l.3113 modale alléger/exclure)
- Test: `client/tests/alleger.test.js` (+ un test dans
  `client/tests/plan_reactivation.test.js` et `client/tests/plan_ajout_run.test.js`)

- [ ] **Step 1 : écrire le test défaillant côté modale**

Dans `alleger.test.js`, sur le modèle de `plan_rampe.test.js` (~l.296-357 :
`plan_load` + `ouvrirPlan()` + `attendreApercu()`) :

```js
const attendreApercu = () => new Promise((r) => setTimeout(r, 320));
const apercus = (ctx) => ctx.invocations.filter(([c]) => c === "plan_preview").length;

test("un retrait réussi recalcule l'aperçu, pas seulement le récap", async () => {
  // Le vécu du 16/08 : après un allègement, Visé/Stock/Placé décrivaient le
  // plan d'avant jusqu'à la prochaine frappe. L'aperçu se périme au moment
  // même où le plan s'écrit : c'est là qu'il se recalcule.
  const ctx = chargerApp();
  const params = {
    runs: [{ num: "RF01", date: "2099-01-05", jjs: [5], exclu: false }],
    debut: "2098-12-01", fin: "2099-03-31", meps: ["2099-01-01"], mep_count: 0,
    cible: null, seed: 7, pa_exclues: [],
    rampe: { forme: "plate", pilote: null },
  };
  const lignes = [ligne("CF1"), ligne("CF2")];
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_load") return { params, fichier: "brm.csv", rapport: "identique" };
    if (cmd === "plan_lignes") return ctx.evaluer(`(${JSON.stringify(lignes)})`);
    if (cmd === "plan_retirer") return ctx.evaluer("([])");
    if (cmd === "plan_preview") return null; // l'aperçu peut échouer, l'appel suffit
    return ctx.evaluer("[]");
  });
  ctx.evaluer("plan").genere = true;

  await ctx.app.ouvrirPlan();
  await attendreApercu();
  const avant = apercus(ctx);

  await ctx.app.ouvrirAllegerRun(ctx.evaluer(`(${JSON.stringify(RUN)})`),
    ctx.evaluer(`(${JSON.stringify(JOUR_FUTUR)})`));
  // Mode prorata : une proposition d'un compte, puis validation avec motif.
  // Reprendre ici la mécanique de proposition/validation des tests existants
  // du mode prorata (champ N, bouton « Proposer », motif, bouton « Retirer »).
  /* … geste complet … */
  await attendreApercu();

  assert.equal(apercus(ctx), avant + 1,
    "le geste doit redemander un aperçu — le récap seul laisse la timeline mentir");
});
```

Si `plan_preview: null` fait échouer le rendu dans le faux DOM, répondre un
aperçu minimal comme `plan_rampe.test.js` (`apercu(...)`) — l'assertion porte
sur l'APPEL, pas sur le rendu.

- [ ] **Step 2 : vérifier l'échec**

Run : `cd client && node --test tests/alleger.test.js 2>&1 | tail -10`
Attendu : ÉCHEC — `apercus` inchangé après le geste.

- [ ] **Step 3 : implémenter l'épilogue commun**

Dans `app.js`, sous `rechargerRecap` (~l.3381) :

```js
/** Épilogue d'une retouche réussie du plan : le récap relit les lignes, et
 *  l'aperçu — qui simule la prochaine régénération — se recalcule. Sans lui,
 *  Visé/Stock/Placé décrivent le plan d'avant le geste jusqu'à la prochaine
 *  frappe (vécu du 16/08). */
async function rechargerApresRetouche() {
  await rechargerRecap();
  planRecalc();
}
```

Remplacer `await rechargerRecap();` par `await rechargerApresRetouche();` aux
**7 sites de retouche uniquement** : ~l.2417 (déplacement), ~l.2451 (retrait
récap), ~l.2506 (réactivation), ~l.2728 et ~l.2799 (rapprochement appliqué —
c'est aussi une retouche du plan), ~l.2991 (ajout), ~l.3113 (modale
alléger/exclure). Les autres appels de `rechargerRecap` (ouverture d'écran,
génération) ne changent pas.

- [ ] **Step 4 : couvrir réactivation et ajout**

Ajouter le même test (adapté au geste du fichier : réactivation dans
`plan_reactivation.test.js`, ajout dans `plan_ajout_run.test.js`), en
réutilisant les harnais de geste déjà présents dans chacun et le duo
`ouvrirPlan()` / `attendreApercu()` ci-dessus. Assertion identique :
`plan_preview` est redemandé après le geste réussi.

- [ ] **Step 5 : suite JS complète**

Run : `cd client && node --test "tests/*.test.js" 2>&1 | tail -5`
Attendu : PASS. ⚠ Ne pas conclure sur un grep de « pass » : lire le résumé
final (`# fail 0`).

- [ ] **Step 6 : commit**

```bash
git add client/src/app.js client/tests/alleger.test.js client/tests/plan_reactivation.test.js client/tests/plan_ajout_run.test.js
git commit -m "feat(superpopaul): l'aperçu du plan se recalcule après chaque retouche"
```

---

## Tâche 6 : Filets de fin de chantier

- [ ] **Step 1 : suites complètes des deux mondes**

Run : `cd client/src-tauri && cargo test 2>&1 | tail -3` puis
`cd client && node --test "tests/*.test.js" 2>&1 | tail -3`
Attendu : tout vert, compter les tests (attendu : ≥ 681 Rust + ≥ 118 JS).

- [ ] **Step 2 : passe de mutation ciblée**

Sauvegarder l'arbre propre d'abord (`git status` doit être vide — leçon
v1.6.0 : sauvegarde de mutation jamais rafraîchie = un test perdu). Muter à la
main, une à la fois, en vérifiant qu'au moins un test tombe :

1. `consomme()` : retirer `+ self.retirees.len()` → les tests de la tâche 1
   doivent tomber.
2. `cible_auto` : retirer `.chain(&preserves.retirees)` → idem.
3. `allouer` : retirer `.chain(&preserves.retirees)` de la semence
   `places_par_pa` → `le_retrait_d_une_auto_ne_deplace_pas_les_quotas_de_plateforme`
   doit tomber.
4. `sauver_apres_retouche` : supprimer l'appel `ecrire_rapport_plan` →
   `une_retouche_reecrit_le_rapport_html_du_plan` doit tomber.
5. `app.js` : `if (gele)` → `if (false)` → le test des trois segments tombe.
6. `rechargerApresRetouche` : supprimer `planRecalc()` → les tests de la
   tâche 5 tombent.

Restaurer après chaque mutation (`git checkout -- <fichier>` restaure depuis
l'INDEX : vérifier que rien n'y est en attente). Toute mutation survivante :
soit écrire le test manquant, soit documenter l'équivalence.

- [ ] **Step 3 : commit final éventuel**

Si la passe de mutation a produit des tests supplémentaires :

```bash
git add -A && git commit -m "test(superpopaul): tests issus de la passe de mutation du chantier retrait"
```
