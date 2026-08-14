# Alléger un run — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** trois aides à la saisie des retraits en lot (exclure un run passé, retirer N au prorata des plateformes, ne garder qu'une sélection), toutes tracées par le mécanisme du PR #2, avec un rapport qui regroupe les retraits par geste.

**Architecture :** les aides calculent des listes de CF (fonctions pures dans `plan.rs`) et aboutissent au `plan::retirer` existant — aucune migration. Le rapport regroupe les retraits manuels par la clé `(le, motif)`, déjà partagée par tout lot passé en un appel. Deux retouches héritées de la revue du PR #2 : « gelé » évalué au moment du geste, et dédoublonnage d'un fragment de bandeau.

**Tech stack :** Rust (Tauri, modules étanches, TDD), vanilla JS (`h()`, jamais d'innerHTML avec des données dynamiques), tests JS `node --test` sur le vrai `app.js` via `client/tests/dom_shim.js`. Spec : `docs/superpowers/specs/2026-08-14-alleger-un-run-design.md`.

**Préalable absolu :** le PR #2 (v1.8.0) doit être **mergé** — ce plan modifie du code qu'il introduit (`retraits_manuels_depuis`, `RetraitManuel`, la section « décision manuelle » du rapport).

---

## Carte des fichiers

- Modifier : `client/src-tauri/src/plan.rs` — `cfs_actifs_du_run`, `proposer_retrait_proportionnel`, verrou « un lot partage son horodatage » (+ tests dans son `mod tests`, ligne ~1200).
- Modifier : `client/src-tauri/src/rapprochement_report.rs` — `GesteManuel`/`CompteRetire` remplacent `RetraitManuel`, rendu regroupé (tableau + alerte), tuile en somme.
- Modifier : `client/src-tauri/src/commands.rs` — `gestes_manuels_depuis` remplace `retraits_manuels_depuis` (regroupement + gelé-au-geste), commande `plan_proposer_retrait`, adaptation de `plan_rapprocher`/`plan_rapprocher_appliquer`.
- Modifier : `client/src-tauri/src/lib.rs:76` — enregistrer `plan_proposer_retrait` dans le `generate_handler!`.
- Modifier : `client/src/app.js` — dédoublonnage du bandeau (`compteRenduRapprochement`, ~l. 2651), bouton « Alléger… » sur la ligne du run (~l. 2026), modale `ouvrirAllegerRun`.
- Créer : `client/tests/alleger.test.js` — câblage de la modale.
- Créer : `docs/superpowers/maquettes/2026-08-14-alleger-un-run.html` et `2026-08-14-rapport-gestes-groupes.html`.
- Créer : `docs/releases/v1.9.0.md`. Modifier : `client/src-tauri/Cargo.toml`, `client/src-tauri/tauri.conf.json` (bump 1.9.0).

---

### Task 1 : Préparation — vérifier le merge du PR #2, brancher

- [ ] **Step 1 : vérifier que le PR #2 est mergé**

Run : `gh pr view 2 --json state --jq .state`
Attendu : `MERGED`. **Sinon : STOP — demander à l'utilisateur.** Ce plan ne s'exécute pas par-dessus un PR #2 ouvert.

- [ ] **Step 2 : brancher depuis main à jour**

```bash
git checkout main && git pull && git checkout -b claude/alleger-un-run
```

- [ ] **Step 3 : baseline verte**

Run : `cd client/src-tauri && cargo test 2>&1 | tail -5` — attendu : 659 tests, 0 échec (le test connu comme instable est `resolver::tests_engine::rafale_5xx_ouvre_le_breaker_une_seule_fois_puis_reprend` ; s'il est le SEUL rouge, relancer une fois).
Run : `cd client && node --test "tests/*.test.js" 2>&1 | tail -3` — attendu : 98 pass.

### Task 2 : Maquettes — **CHECKPOINT UTILISATEUR**

Règle projet : maquette HTML validée par un **go explicite** avant tout code UI. Les tâches 8 et 9 (UI) sont bloquées tant que ce go n'est pas donné.

- [ ] **Step 1 : maquette de la modale** `docs/superpowers/maquettes/2026-08-14-alleger-un-run.html`

Palette réelle « Bleu nuit & or » (reprendre les variables CSS d'une maquette existante, ex. `2026-08-14-rapport-retraits-manuels.html`). Trois états côte à côte :
1. **Run passé** : titre « Alléger le run RF02 », texte « Run du 28/07 — 143 comptes actifs », un seul mode « Exclure le run », motif pré-rempli « Run RF02 du 28/07/2026 exclu a posteriori — » (textarea), bouton danger « Retirer 143 compte(s) » inerte tant que le motif n'est pas complété au-delà du pré-remplissage, avertissement MEP gelée existant.
2. **Run à venir, mode proportionnel** : champ N, proposition groupée par plateforme (« Esalink — 4 sur 12 » + comptes proposés, chacun échangeable via un lien « échanger » ouvrant la liste des autres comptes actifs de la même plateforme), motif, bouton « Retirer N compte(s) ».
3. **Run à venir, mode sélection** : liste des comptes actifs avec cases sur les **gardés**, pied permanent « 2 gardé(s) — 5 seront retiré(s) », motif, bouton « Retirer 5 compte(s) ».

- [ ] **Step 2 : maquette du rapport regroupé** `docs/superpowers/maquettes/2026-08-14-rapport-gestes-groupes.html`

Reprendre le rendu du rapport de rapprochement (mêmes CSS que `rapprochement_report.rs`) et montrer : le tableau « Comptes retirés — décision manuelle » avec un geste de 3 comptes sous chapeau (« 3 comptes retirés le 31/07/2026 — Motif : … ») + un retrait isolé rendu comme aujourd'hui ; l'alerte rouge avec une entrée agrégée (« 143 comptes retirés le 14/08/2026 figuraient dans des fichiers déjà transmis… ») + une entrée unitaire.

- [ ] **Step 3 : commit + STOP pour validation**

```bash
git add docs/superpowers/maquettes/ && git commit -m "docs(superpopaul): maquettes alléger un run + rapport groupé"
```
**STOP : montrer les maquettes (serve.sh) et attendre le go explicite de l'utilisateur avant les tâches 8 et 9.** Les tâches 3 à 7 (Rust pur) peuvent avancer pendant l'attente.

### Task 3 : `plan.rs` — `cfs_actifs_du_run` + verrou d'horodatage (TDD)

**Files :** Modify `client/src-tauri/src/plan.rs` (fonctions près de `retirer`, l. ~927 ; tests dans `mod tests`, l. ~1200).

- [ ] **Step 1 : tests rouges**

Dans `mod tests` de `plan.rs`, ajouter (adapter le constructeur si un builder de `LignePlan` existe déjà dans ce module — le réutiliser au lieu de dupliquer) :

```rust
fn l_run(cf: &str, run: &str, pa: &str, origine: Origine, in_dir: bool, res_at: i64) -> LignePlan {
    LignePlan {
        cf: cf.into(),
        participant: format!("0225:{cf}"),
        jj: 5,
        raison_sociale: String::new(),
        pa: pa.into(),
        mep_id: 1,
        mep_date: chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        run_num: run.into(),
        run_date: chrono::NaiveDate::from_ymd_opt(2026, 8, 20).unwrap(),
        origine,
        in_directory: in_dir,
        resolved_at: res_at,
        planned_at: 0,
        retire: None,
    }
}

#[test]
fn cfs_actifs_du_run_exclut_les_retirees_et_les_autres_runs() {
    let mut retiree = l_run("CF2", "RF01", "Esalink", Origine::Auto, true, 10);
    retiree.retire = Some(Retrait { le: 1, motif: "déjà sorti".into() });
    let plan = vec![
        l_run("CF1", "RF01", "Esalink", Origine::Auto, true, 10),
        retiree,
        l_run("CF3", "RF02", "Esalink", Origine::Auto, true, 10),
        // Origines confondues : exclure un run, c'est TOUT le run, épinglées comprises.
        l_run("CF4", "RF01", "Serensia", Origine::Manuel, true, 10),
    ];
    assert_eq!(cfs_actifs_du_run(&plan, "RF01"), vec!["CF1", "CF4"]);
}

#[test]
fn un_retrait_en_lot_pose_le_meme_horodatage_sur_toutes_ses_lignes() {
    // VERROU DU REGROUPEMENT. Le rapport regroupe les retraits manuels par
    // (le, motif) : un geste n'existe comme geste QUE parce que `retirer`
    // reçoit un seul `maintenant` pour tout le lot. Si l'horloge se mettait à
    // être lue par ligne, un run exclu deviendrait 143 gestes d'un compte.
    let mut plan = vec![
        l_run("CF1", "RF01", "Esalink", Origine::Auto, true, 10),
        l_run("CF2", "RF01", "Esalink", Origine::Auto, true, 10),
        l_run("CF3", "RF01", "Serensia", Origine::Auto, true, 10),
    ];
    retirer(&mut plan, &["CF1".into(), "CF2".into(), "CF3".into()], "run exclu", 1_786_017_600)
        .unwrap();
    assert!(plan.iter().all(|l| l.retire.as_ref().unwrap().le == 1_786_017_600));
}
```

- [ ] **Step 2 : vérifier l'échec** — `cargo test cfs_actifs_du_run` → erreur de compilation (`cfs_actifs_du_run` inconnue) : c'est le rouge attendu. Le test du verrou, lui, doit déjà passer (le comportement existe) — il fige l'invariant.

- [ ] **Step 3 : implémentation minimale** (sous `annuler_retrait`, l. ~967)

```rust
/// Les comptes actifs d'un run, toutes origines confondues — la matière des
/// gestes en lot (« exclure le run », « ne garder que… »). Les retirées sont
/// exclues : on ne retire pas deux fois.
pub fn cfs_actifs_du_run(plan: &[LignePlan], run_num: &str) -> Vec<String> {
    plan.iter()
        .filter(|l| l.run_num == run_num && !l.retiree())
        .map(|l| l.cf.clone())
        .collect()
}
```

- [ ] **Step 4 : vert** — `cargo test cfs_actifs_du_run un_retrait_en_lot` → 2 pass.
- [ ] **Step 5 : commit** — `git add -A && git commit -m "feat(superpopaul): les comptes actifs d'un run, et le verrou de l'horodatage de lot"`

### Task 4 : `plan.rs` — `proposer_retrait_proportionnel` (TDD)

**Files :** Modify `client/src-tauri/src/plan.rs` (implémentation près de `quotas_par_pa` l. ~307, dont elle est le miroir ; tests dans `mod tests`).

- [ ] **Step 1 : tests rouges**

```rust
#[test]
fn la_repartition_du_retrait_suit_les_effectifs_du_run() {
    // 12 Esalink + 6 Serensia, retirer 6 → 4 + 2 : la distribution RESTANTE
    // garde les proportions du run. Miroir de `quotas_par_pa`.
    let mut plan = Vec::new();
    for i in 0..12 { plan.push(l_run(&format!("E{i:02}"), "RF01", "Esalink", Origine::Auto, true, 10)); }
    for i in 0..6 { plan.push(l_run(&format!("S{i:02}"), "RF01", "Serensia", Origine::Auto, true, 10)); }
    let cfs = proposer_retrait_proportionnel(&plan, "RF01", 6, 42).unwrap();
    assert_eq!(cfs.len(), 6);
    assert_eq!(cfs.iter().filter(|c| c.starts_with('E')).count(), 4);
    assert_eq!(cfs.iter().filter(|c| c.starts_with('S')).count(), 2);
}

#[test]
fn jamais_le_dernier_compte_d_une_plateforme() {
    // Plancher 1 inversé : la couverture gagnée à la génération ne se perd
    // pas par décimage. Ici Serensia n'a qu'un compte : tout sort d'Esalink.
    let plan = vec![
        l_run("E1", "RF01", "Esalink", Origine::Auto, true, 10),
        l_run("E2", "RF01", "Esalink", Origine::Auto, true, 10),
        l_run("E3", "RF01", "Esalink", Origine::Auto, true, 10),
        l_run("S1", "RF01", "Serensia", Origine::Auto, true, 10),
    ];
    let cfs = proposer_retrait_proportionnel(&plan, "RF01", 2, 42).unwrap();
    assert!(cfs.iter().all(|c| c.starts_with('E')), "S1 est le dernier Serensia : intouchable");
}

#[test]
fn n_au_dela_du_maximum_dit_le_maximum() {
    let plan = vec![
        l_run("E1", "RF01", "Esalink", Origine::Auto, true, 10),
        l_run("E2", "RF01", "Esalink", Origine::Auto, true, 10),
        l_run("S1", "RF01", "Serensia", Origine::Auto, true, 10),
    ];
    // max = (2−1) + (1−1) = 1.
    let err = proposer_retrait_proportionnel(&plan, "RF01", 2, 42).unwrap_err();
    assert!(err.contains("maximum est 1"), "le message doit dire le maximum retirable : {err}");
}

#[test]
fn zero_et_run_vide_sont_refuses() {
    let plan = vec![l_run("E1", "RF01", "Esalink", Origine::Auto, true, 10)];
    assert!(proposer_retrait_proportionnel(&plan, "RF01", 0, 42).is_err());
    assert!(proposer_retrait_proportionnel(&plan, "RF99", 1, 42).is_err());
}

#[test]
fn les_protegees_sortent_en_dernier() {
    // Couverture et épinglées ne partent qu'en dernier recours : elles portent
    // une décision (représentation d'une PA, geste humain) que le décimage
    // automatique n'a pas à défaire tant que des allouées suffisent.
    let plan = vec![
        l_run("A1", "RF01", "Esalink", Origine::Auto, true, 10),
        l_run("C1", "RF01", "Esalink", Origine::Couverture, true, 10),
        l_run("M1", "RF01", "Esalink", Origine::Manuel, true, 10),
        l_run("A2", "RF01", "Esalink", Origine::Auto, true, 10),
    ];
    let cfs = proposer_retrait_proportionnel(&plan, "RF01", 2, 42).unwrap();
    assert_eq!({ let mut v = cfs.clone(); v.sort(); v }, vec!["A1", "A2"]);
}

#[test]
fn l_ordre_de_sortie_est_l_inverse_de_la_priorite_d_allocation() {
    // Sortent d'abord : hors annuaire, puis résolution la plus ancienne —
    // exactement ce que `trier_par_priorite` place en queue à la génération.
    let plan = vec![
        l_run("FRAIS", "RF01", "Esalink", Origine::Auto, true, 100),
        l_run("VIEUX", "RF01", "Esalink", Origine::Auto, true, 1),
        l_run("HORS", "RF01", "Esalink", Origine::Auto, false, 100),
    ];
    assert_eq!(proposer_retrait_proportionnel(&plan, "RF01", 1, 42).unwrap(), vec!["HORS"]);
    assert_eq!(proposer_retrait_proportionnel(&plan, "RF01", 2, 42).unwrap(), vec!["HORS", "VIEUX"]);
}
```

- [ ] **Step 2 : vérifier l'échec** — `cargo test proposer_retrait` → compilation rouge (fonction inconnue).

- [ ] **Step 3 : implémentation** (sous `cfs_actifs_du_run`)

```rust
/// Propose N comptes à retirer d'un run en conservant la distribution des
/// plateformes — le miroir de `quotas_par_pa` : mêmes plus forts restes, même
/// boucle de plafond, mais à l'envers (on décime au lieu de doter).
///
/// Règles :
/// - **plancher 1 inversé** : jamais le dernier compte actif d'une plateforme —
///   pour tout retirer, c'est l'exclusion du run, pas le décimage ;
/// - **protections** : `Couverture` et `Manuel` ne sortent qu'en dernier
///   recours, quand les allouées de la plateforme ne suffisent pas ;
/// - **ordre de sortie** : l'inverse de `trier_par_priorite` — hors annuaire
///   d'abord, puis résolutions les plus anciennes, départage seedé (même seed
///   que la génération : la proposition est reproductible d'un clic à l'autre).
pub fn proposer_retrait_proportionnel(
    plan: &[LignePlan],
    run_num: &str,
    n: usize,
    seed: u64,
) -> Result<Vec<String>, String> {
    let mut par_pa: BTreeMap<&str, Vec<&LignePlan>> = BTreeMap::new();
    for l in plan.iter().filter(|l| l.run_num == run_num && !l.retiree()) {
        par_pa.entry(l.pa.as_str()).or_default().push(l);
    }
    if par_pa.is_empty() {
        return Err(format!("aucun compte actif sur le run « {run_num} »"));
    }
    if n == 0 {
        return Err("rien à retirer : N doit être au moins 1".into());
    }
    let max: usize = par_pa.values().map(|v| v.len() - 1).sum();
    if n > max {
        return Err(format!(
            "impossible de retirer {n} compte(s) : le maximum est {max} — chaque \
             plateforme du run garde au moins un compte. Pour tout retirer, \
             c'est l'exclusion du run."
        ));
    }
    // Répartition aux plus forts restes, pondérée par les effectifs actifs.
    let poids: BTreeMap<String, f64> =
        par_pa.iter().map(|(pa, v)| ((*pa).to_string(), v.len() as f64)).collect();
    let mut quotas = plus_forts_restes(n, &poids);
    // Plafond au retirable (effectif − 1), l'excédent repart vers les
    // plateformes qui ont de la marge — la boucle de `quotas_par_pa`, inversée.
    loop {
        let mut excedent = 0usize;
        let mut place: BTreeMap<String, f64> = BTreeMap::new();
        for (pa, v) in &par_pa {
            let plafond = v.len() - 1;
            let q = quotas.entry((*pa).to_string()).or_insert(0);
            if *q > plafond {
                excedent += *q - plafond;
                *q = plafond;
            } else if *q < plafond {
                place.insert((*pa).to_string(), (plafond - *q) as f64);
            }
        }
        if excedent == 0 {
            break;
        }
        for (pa, x) in plus_forts_restes(excedent, &place) {
            *quotas.get_mut(&pa).expect("clé issue de place") += x;
        }
    }
    let mut out = Vec::new();
    for (pa, mut v) in par_pa {
        let rang = |l: &LignePlan| match l.origine {
            Origine::Auto => 0u8,
            Origine::Couverture => 1,
            Origine::Manuel => 2,
        };
        v.sort_by(|a, b| {
            rang(a)
                .cmp(&rang(b))
                .then_with(|| a.in_directory.cmp(&b.in_directory))
                .then_with(|| a.resolved_at.cmp(&b.resolved_at))
                .then_with(|| hash_seede(seed, &a.cf).cmp(&hash_seede(seed, &b.cf)))
        });
        out.extend(v.into_iter().take(quotas.get(pa).copied().unwrap_or(0)).map(|l| l.cf.clone()));
    }
    Ok(out)
}
```

Note : `l_ordre_de_sortie…` attend `["HORS", "VIEUX"]` dans cet ordre parce que la sortie suit l'ordre de tri intra-PA. Si `plus_forts_restes` impose une précondition (voir son commentaire l. ~237-269), la respecter comme `quotas_par_pa` le fait.

- [ ] **Step 4 : vert** — `cargo test proposer_retrait` → 6 pass.
- [ ] **Step 5 : commit** — `git commit -am "feat(superpopaul): proposition de retrait proportionnel par plateforme"`

### Task 5 : rapport — le geste remplace le retrait isolé (TDD)

**Files :** Modify `client/src-tauri/src/rapprochement_report.rs` (types l. ~36-53, rendu l. ~232-390, tests l. ~580+) ; Modify `client/src-tauri/src/commands.rs` (producteur, l. ~1797-2050, et ses tests l. ~2472+). Une seule tâche : types et producteur sont indissociables à la compilation.

- [ ] **Step 1 : remplacer les types** dans `rapprochement_report.rs`

`RetraitManuel` et le champ `retraits_manuels` disparaissent au profit de :

```rust
/// Un compte au sein d'un geste. `gelee` est évalué **au moment du geste**
/// (la MEP était-elle passée quand le retrait a été décidé ?) — aligné sur les
/// écarts calculés, qui jugent au moment de la décision. Retouche issue de la
/// revue du PR #2 : évaluer au moment du rapport faisait basculer en alerte
/// rouge des comptes retirés d'une MEP alors future.
pub struct CompteRetire {
    pub cf: String,
    pub gelee: bool,
}

/// Un geste de retrait manuel : ce que l'utilisateur a fait en un clic.
/// La clé (date, motif) vient du producteur — un lot passé par `plan::retirer`
/// partage son horodatage (verrou dans `plan::tests`) et son motif.
pub struct GesteManuel {
    /// Date ISO du geste (jour local du retrait). Rendue via `date_fr`.
    pub le: String,
    /// Saisi par l'utilisateur. **Texte libre**, échappé au point d'insertion.
    pub motif: String,
    /// Triés par n° de CF par le producteur.
    pub comptes: Vec<CompteRetire>,
}
```

Dans `RapprochementReportData` : `pub gestes_manuels: &'a [GesteManuel],` remplace `retraits_manuels` (le champ `depuis` ne change pas).

- [ ] **Step 2 : adapter les tests existants du PR #2 et écrire les nouveaux** (dans `mod tests` de `rapprochement_report.rs`)

Remplacer le helper `retrait_manuel` par :

```rust
fn geste(le: &str, motif: &str, comptes: &[(&str, bool)]) -> GesteManuel {
    GesteManuel {
        le: le.into(),
        motif: motif.into(),
        comptes: comptes.iter().map(|(cf, g)| CompteRetire { cf: (*cf).into(), gelee: *g }).collect(),
    }
}
```

Les tests du PR #2 s'adaptent mécaniquement : un ancien `retrait_manuel(cf, le, motif, gelee)` devient `geste(le, motif, &[(cf, gelee)])`, et `d.retraits_manuels = &manuels` devient `d.gestes_manuels = &gestes`. Leur intention ne change pas (tuile, sous-titre `Depuis`, échappement, alerte unitaire, arithmétique préservée — la tuile compte désormais `Σ comptes`). Nouveaux tests :

```rust
#[test]
fn un_geste_de_plusieurs_comptes_est_rendu_sous_chapeau() {
    let r = vide();
    let gestes = vec![geste("2026-07-31", "Run RF02 du 28/07/2026 exclu a posteriori — erreurs",
        &[("4100238091", false), ("4100243662", false), ("4100247788", false)])];
    let mut d = donnees(&r);
    d.gestes_manuels = &gestes;
    let html = render(&d);
    let c = corps(&html);
    assert!(c.contains("<b>3</b> comptes retirés le 31/07/2026"), "chapeau absent");
    // Le motif se lit UNE fois (au chapeau), pas trois.
    assert_eq!(c.matches("exclu a posteriori").count(), 1, "le motif ne doit pas se répéter");
    for cf in ["4100238091", "4100243662", "4100247788"] {
        assert!(c.contains(cf), "chaque compte du geste doit rester pointable : {cf}");
    }
}

#[test]
fn un_geste_d_un_seul_compte_garde_le_rendu_du_pr_2() {
    let r = vide();
    let gestes = vec![geste("2026-07-31", "litige", &[("4100238091", false)])];
    let mut d = donnees(&r);
    d.gestes_manuels = &gestes;
    let c = corps(&render(&d));
    assert!(c.contains("31/07/2026"));
    assert!(!c.contains("comptes retirés le"), "pas de chapeau pour un geste d'un compte");
}

#[test]
fn l_alerte_agrege_les_geles_d_un_meme_geste() {
    // 150 lignes d'alerte pour un seul clic noieraient la seule information
    // qui oblige le destinataire à agir. Une entrée par geste ; la liste
    // exacte reste au tableau.
    let r = vide();
    let gestes = vec![
        geste("2026-08-14", "Run RF02 exclu a posteriori — erreurs",
            &[("4100238091", true), ("4100243662", true), ("4100247788", false)]),
        geste("2026-08-06", "litige", &[("4100250000", true)]),
    ];
    let mut d = donnees(&r);
    d.gestes_manuels = &gestes;
    let c = corps(&render(&d));
    let alerte = c.split(T_ALERTE).nth(1).expect("alerte absente")
        .split("</section>").next().unwrap_or("");
    assert!(alerte.contains("<b>2</b> comptes retirés le 14/08/2026"),
        "les gelés du geste s'agrègent (2 sur 3 : le non-gelé n'y entre pas)");
    assert!(!alerte.contains("4100238091"), "pas d'énumération par compte pour un geste multiple");
    assert!(alerte.contains("4100250000"), "un geste d'un seul gelé garde la phrase unitaire");
}
```

- [ ] **Step 3 : adapter le rendu** dans `render()`

Tuile (l. ~239) : `let total_manuels: usize = d.gestes_manuels.iter().map(|g| g.comptes.len()).sum();` — la tuile se rend si `total_manuels > 0` et affiche `fmt_int(total_manuels as u64)` (elle compte des comptes, pas des gestes).

Alerte (l. ~275-308) — remplacer le bloc `geles_manuels` :

```rust
    // Une entrée par GESTE contenant des gelés : la mise en évidence doit
    // rester lisible quand un run entier sort — la liste exacte est au tableau.
    let gestes_geles: Vec<(&GesteManuel, usize)> = d
        .gestes_manuels
        .iter()
        .filter_map(|g| {
            let n = g.comptes.iter().filter(|c| c.gelee).count();
            (n > 0).then_some((g, n))
        })
        .collect();
    if !geles.is_empty() || !gestes_geles.is_empty() {
        // … (ouverture de section inchangée, puis les <li> calculés existants)
        for (g, n) in &gestes_geles {
            if *n == 1 {
                let cf = &g.comptes.iter().find(|c| c.gelee).expect("n == 1").cf;
                html.push_str(&format!(
                    "<li>Le compte <b>{}</b> figurait dans un fichier qui vous a déjà été \
                     transmis. Les fichiers étant cumulatifs, il ne figure plus dans aucun \
                     fichier de ce lot. Motif : <b>{}</b>.</li>\n",
                    esc(cf), esc(&g.motif),
                ));
            } else {
                html.push_str(&format!(
                    "<li><b>{}</b> comptes retirés le {} figuraient dans des fichiers qui vous \
                     ont déjà été transmis. Les fichiers étant cumulatifs, ils ne figurent plus \
                     dans aucun fichier de ce lot — la liste est au tableau « Comptes retirés — \
                     décision manuelle ». Motif : <b>{}</b>.</li>\n",
                    fmt_int(*n as u64), esc(&date_fr(&g.le)), esc(&g.motif),
                ));
            }
        }
        html.push_str("</ul>\n</section>\n");
    }
```

Tableau (l. ~350-385) — remplacer la boucle `for m in d.retraits_manuels` :

```rust
    for g in d.gestes_manuels {
        if g.comptes.len() == 1 {
            html.push_str(&format!(
                "<tr><td>{}</td><td class=\"date\">{}</td><td>{}</td></tr>\n",
                esc(&g.comptes[0].cf), esc(&date_fr(&g.le)), esc(&g.motif),
            ));
        } else {
            // Le chapeau porte la date et le motif UNE fois ; les lignes du
            // geste ne portent que le compte — c'est la décision qui se lit,
            // pas 150 répétitions.
            html.push_str(&format!(
                "<tr class=\"geste\"><td colspan=\"3\"><b>{}</b> comptes retirés le {} — \
                 Motif : <b>{}</b></td></tr>\n",
                fmt_int(g.comptes.len() as u64), esc(&date_fr(&g.le)), esc(&g.motif),
            ));
            for c in &g.comptes {
                html.push_str(&format!(
                    "<tr class=\"du-geste\"><td>{}</td><td class=\"date\"></td><td></td></tr>\n",
                    esc(&c.cf),
                ));
            }
        }
    }
    fin_section(&mut html, d.gestes_manuels.is_empty());
```

CSS (`CSS_RAPPRO`, l. ~547) : ajouter (à ajuster sur la maquette validée)

```css
  tr.geste td { background: rgba(255, 255, 255, .04); font-size: 13px; }
  tr.du-geste td { color: var(--muted); }
```

Le sous-titre `Depuis` et l'ordre des sections ne changent pas. Vide : `d.gestes_manuels.is_empty()` partout où `retraits_manuels.is_empty()` figurait.

- [ ] **Step 4 : adapter le producteur** dans `commands.rs`

Scinder `jour_local_iso` (l. ~1804) :

```rust
fn jour_local(ts: i64) -> chrono::NaiveDate {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
        .unwrap_or_default()
}

fn jour_local_iso(ts: i64) -> String {
    jour_local(ts).to_string()
}
```

Remplacer `retraits_manuels_depuis` par (conserver le commentaire de doc du filigrane du PR #2, complété du regroupement) :

```rust
fn gestes_manuels_depuis(
    lignes: &[crate::plan::LignePlan],
    rapproche_le: Option<i64>,
) -> Vec<crate::rapprochement_report::GesteManuel> {
    let seuil = rapproche_le.unwrap_or(i64::MIN);
    // Clé du geste : (horodatage BRUT à la seconde, motif). Un lot passé par
    // `plan::retirer` partage les deux (verrou dans `plan::tests`). Le BTreeMap
    // rend l'ordre : par date de geste, puis motif.
    let mut groupes: std::collections::BTreeMap<(i64, String), Vec<&crate::plan::LignePlan>> =
        Default::default();
    for l in lignes {
        let Some(r) = l.retire.as_ref() else { continue };
        if r.le <= seuil {
            continue;
        }
        groupes.entry((r.le, r.motif.clone())).or_default().push(l);
    }
    groupes
        .into_iter()
        .map(|((le, motif), mut ls)| {
            ls.sort_by(|a, b| a.cf.cmp(&b.cf));
            crate::rapprochement_report::GesteManuel {
                le: jour_local_iso(le),
                motif,
                comptes: ls
                    .into_iter()
                    .map(|l| crate::rapprochement_report::CompteRetire {
                        cf: l.cf.clone(),
                        // Gelé AU MOMENT DU GESTE : la MEP était déjà passée
                        // quand le retrait a été décidé.
                        gelee: l.mep_date < jour_local(le),
                    })
                    .collect(),
            }
        })
        .collect()
}
```

Le paramètre `aujourdhui` disparaît. Call sites :
- `plan_rapprocher` (l. ~1923) : `let retraits_manuels = gestes_manuels_depuis(&lignes, meta.rapproche_le).iter().map(|g| g.comptes.len()).sum();` (le champ `RapprochementVue.retraits_manuels: usize` ne change pas — le JS non plus).
- `plan_rapprocher_appliquer` (l. ~1981) : `let gestes_manuels = gestes_manuels_depuis(&lignes, meta.rapproche_le);` et `gestes_manuels: &gestes_manuels,` dans `RapprochementReportData` (les commentaires de capture AVANT le réalignement de `meta` restent valables tels quels).

- [ ] **Step 5 : adapter les tests du producteur** (tests `commands.rs`, l. ~2472+)

Les 7 tests du PR #2 migrent vers la nouvelle API — même intention, forme groupée. Exemples des cas qui changent de fond :

```rust
#[test]
fn un_retrait_anterieur_a_sa_mep_n_est_pas_gele_meme_si_la_mep_est_passee_depuis() {
    // LE GESTE, PAS LE RAPPORT : retiré le 31/07 d'une MEP du 06/08 — au
    // moment de la décision, aucun fichier transmis ne devenait faux. Que le
    // rapport soit produit le 14/08 n'y change rien. (Retouche revue PR #2.)
    let lignes = vec![ligne_retiree("4100238091", "2026-08-06", LE_31_JUILLET, "périmètre 2027")];
    let g = gestes_manuels_depuis(&lignes, None);
    assert!(!g[0].comptes[0].gelee);
}

#[test]
fn un_retrait_posterieur_a_sa_mep_est_gele() {
    // MEP du 12/06, retrait le 06/08 : le fichier transmis contenait le compte.
    let lignes = vec![ligne_retiree("4100247788", "2026-06-12", LE_6_AOUT, "litige")];
    let g = gestes_manuels_depuis(&lignes, None);
    assert!(g[0].comptes[0].gelee);
}

#[test]
fn deux_retraits_du_meme_lot_forment_un_seul_geste() {
    let lignes = vec![
        ligne_retiree("4100243662", "2026-12-01", LE_31_JUILLET, "comité"),
        ligne_retiree("4100238091", "2026-12-01", LE_31_JUILLET, "comité"),
    ];
    let g = gestes_manuels_depuis(&lignes, None);
    assert_eq!(g.len(), 1);
    let cfs: Vec<&str> = g[0].comptes.iter().map(|c| c.cf.as_str()).collect();
    assert_eq!(cfs, vec!["4100238091", "4100243662"], "triés par compte dans le geste");
}

#[test]
fn meme_seconde_mais_motifs_differents_font_deux_gestes() {
    // La collision d'une seconde entre DEUX gestes réels est assumée (comme
    // celle du filigrane) — mais si les motifs diffèrent, ce sont bien deux
    // décisions, et le document ne doit pas les fondre.
    let lignes = vec![
        ligne_retiree("4100238091", "2026-12-01", LE_31_JUILLET, "comité"),
        ligne_retiree("4100243662", "2026-12-01", LE_31_JUILLET, "litige"),
    ];
    assert_eq!(gestes_manuels_depuis(&lignes, None).len(), 2);
}
```

Les autres (`…apres_le_dernier_rapprochement_est_liste`, `…par_le_rapprochement_lui_meme_n_est_pas_liste` — le verrou du filigrane, dont le commentaire est conservé —, `…anterieur_au_dernier_rapprochement…`, `sans_rapprochement_anterieur…`, `une_ligne_active…`, l'ordre) migrent en gardant leurs assertions, adaptées à `g[i].comptes[j]`. Les constantes midi-UTC restent (elles rendent `jour_local` indépendant du fuseau de la machine) ; `jour_de` disparaît avec le paramètre `aujourdhui` si plus rien ne l'utilise — pas de code mort.

- [ ] **Step 6 : tout vert** — `cargo test` → 0 échec (compter : les 659 d'avant + nouveaux − rien de supprimé sans remplaçant).
- [ ] **Step 7 : commit** — `git commit -am "feat(superpopaul): le rapport regroupe les retraits manuels par geste, gelé au moment du geste"`

### Task 6 : commande `plan_proposer_retrait`

**Files :** Modify `client/src-tauri/src/commands.rs` (sous `plan_annuler_retrait`, l. ~1737) ; Modify `client/src-tauri/src/lib.rs` (l. ~110).

- [ ] **Step 1 : test rouge** (tests `commands.rs` — la commande est une enveloppe, le cœur est déjà testé ; on teste le regroupement d'affichage)

```rust
#[test]
fn la_proposition_est_groupee_par_plateforme_avec_les_effectifs() {
    let plan = vec![
        ligne_pa("E1", "RF01", "Esalink"),
        ligne_pa("E2", "RF01", "Esalink"),
        ligne_pa("S1", "RF01", "Serensia"),
        ligne_pa("S2", "RF01", "Serensia"),
    ];
    let props = grouper_proposition(&plan, "RF01", &["E1".into(), "S2".into()]);
    assert_eq!(props.len(), 2);
    assert_eq!((props[0].pa.as_str(), props[0].actifs, props[0].retirer.as_slice()),
        ("Esalink", 2, ["E1".to_string()].as_slice()));
    assert_eq!((props[1].pa.as_str(), props[1].actifs), ("Serensia", 2));
}
```

(`ligne_pa` : mini-builder local réutilisant celui des tests existants de `commands.rs` s'il y en a un — sinon copie de `l_run` de `plan.rs` réduite à cf/run/pa.)

- [ ] **Step 2 : implémentation**

```rust
/// Proposition de retrait proportionnel, groupée par plateforme pour l'écran.
#[derive(Serialize)]
pub struct PropositionPa {
    pub pa: String,
    /// Comptes proposés au retrait, dans l'ordre de sortie.
    pub retirer: Vec<String>,
    /// Effectif actif de la plateforme sur ce run — le « 4 sur 12 » de l'écran.
    pub actifs: usize,
}

/// Regroupement d'affichage, séparé de la commande pour être testable sans
/// `tauri::State` — même motif que `retraits_manuels_depuis` au PR #2.
fn grouper_proposition(
    lignes: &[crate::plan::LignePlan],
    run_num: &str,
    cfs: &[String],
) -> Vec<PropositionPa> {
    let mut par_pa: std::collections::BTreeMap<String, PropositionPa> = Default::default();
    for l in lignes.iter().filter(|l| l.run_num == run_num && !l.retiree()) {
        par_pa
            .entry(l.pa.clone())
            .or_insert_with(|| PropositionPa { pa: l.pa.clone(), retirer: Vec::new(), actifs: 0 })
            .actifs += 1;
    }
    for cf in cfs {
        let pa = &lignes.iter().find(|l| &l.cf == cf).expect("cf issu du plan").pa;
        par_pa.get_mut(pa).expect("pa issue du plan").retirer.push(cf.clone());
    }
    par_pa.into_values().collect()
}

#[tauri::command]
pub async fn plan_proposer_retrait(
    state: State<'_, AppState>,
    run_num: String,
    n: usize,
) -> Result<Vec<PropositionPa>, String> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (lignes, meta) = charger_pour_retouche(&store)?;
        // Même seed que la génération : proposition reproductible.
        let seed = crate::plan::PlanParams::depuis_yaml(&meta.params_yaml)?.seed;
        let cfs = crate::plan::proposer_retrait_proportionnel(&lignes, &run_num, n, seed)?;
        Ok(grouper_proposition(&lignes, &run_num, &cfs))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

Dans `lib.rs`, ajouter `commands::plan_proposer_retrait,` à la liste du `generate_handler!` (l. ~110, à côté de `plan_retirer`).

- [ ] **Step 3 : vert** — `cargo test grouper_proposition && cargo check`.
- [ ] **Step 4 : commit** — `git commit -am "feat(superpopaul): commande de proposition de retrait proportionnel"`

### Task 7 : bandeau — dédoublonner le fragment (revue PR #2)

**Files :** Modify `client/src/app.js` (`compteRenduRapprochement`, l. ~2651).

- [ ] **Step 1 : remplacer la construction du texte**

```js
  const doc = `${fmtN(retraitsManuels)} retrait(s) manuel(s) documenté(s)`;
  let texte = parts.length
    ? `✓ Rapprochement appliqué : ${parts.join(", ")}.` + (retraitsManuels ? ` ${doc}.` : "")
    : retraitsManuels
      ? `✓ Note de livraison produite : ${doc}, aucun compte modifié.`
      : "✓ Rapprochement appliqué.";
```

(Les chaînes produites sont identiques à l'existant : les tests JS du PR #2 doivent rester verts sans retouche.)

- [ ] **Step 2 : vert** — `cd client && node --test "tests/*.test.js"` → 98 pass.
- [ ] **Step 3 : commit** — `git commit -am "fix(superpopaul): le bandeau construit une seule fois la phrase des retraits documentés"`

### Task 8 : modale « Alléger… » (⛔ après le go maquette de la Task 2)

**Files :** Modify `client/src/app.js` (nouvelle fonction sous `ouvrirAjoutRun`, l. ~2793+) ; Test `client/tests/alleger.test.js` (créer). La structure DOM exacte (classes, libellés) suit la **maquette validée** — le câblage ci-dessous est le contrat testé.

- [ ] **Step 1 : tests rouges** — `client/tests/alleger.test.js`

```js
// Modale « Alléger un run » : le cœur (listes de CF, prorata) est testé côté
// Rust — ici, le câblage seulement : les modes offerts selon la date du run,
// le motif obligatoire au-delà du pré-remplissage, le compteur gardés/retirés,
// et UN SEUL plan_retirer par geste (c'est ce qui fait le geste au rapport).
const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

function ligne(cf, extra = {}) {
  return {
    cf, participant: `0225:${cf}`, raison_sociale: `Société ${cf}`,
    jj: 5, pa: "Cegedim", mep_id: 1, mep_date: "2026-09-01", run_num: "RF01",
    run_date: "2026-09-10", origine: "auto", etat: "eligible", gelee: false,
    retire_motif: null, ...extra,
  };
}

/** Un plan généré de 3 comptes actifs + 1 retiré sur RF01. */
function ecran() {
  const ctx = chargerApp();
  const p = ctx.evaluer("plan");
  p.genere = true;
  p.lignes = ctx.evaluer(`(${JSON.stringify([
    ligne("CF1"), ligne("CF2", { pa: "Esalink" }), ligne("CF3"),
    ligne("CF9", { retire_motif: "déjà sorti" }),
  ])})`);
  return ctx;
}

const boutonModale = ($, debut) => trouver($("modal"),
  (n) => n.tagName === "button" && String(n.children[0] ?? "").startsWith(debut));

test("run passé : seul le mode « exclure » est offert, motif à compléter", async () => {
  const ctx = ecran();
  ctx.repondreAux(() => null);
  // Exposé comme renderPlanRecap l'est (via le registre de dom_shim).
  await ctx.app.ouvrirAllegerRun({ num: "RF01", jjs: [5] }, { date: "2026-01-05" });
  const btn = boutonModale(ctx.$, "Retirer 3");
  assert.ok(btn, "l'exclusion porte le compte des actifs (3, pas 4 : CF9 est déjà retiré)");
  assert.equal(btn.disabled, true, "le pré-remplissage ne suffit pas : il faut une cause");
  const zone = trouver(ctx.$("modal"), (n) => n.tagName === "textarea");
  assert.match(zone.value, /Run RF01 .* exclu a posteriori/);
  zone.value += "erreurs détectées au premier run";
  zone.dispatchEvent("input");
  assert.equal(btn.disabled, false);
  await btn.click();
  const appels = ctx.invocations.filter(([c]) => c === "plan_retirer");
  assert.equal(appels.length, 1, "UN SEUL appel : c'est lui qui fait le geste au rapport");
  assert.deepEqual([...appels[0][1].cfs].sort(), ["CF1", "CF2", "CF3"]);
});

test("run à venir, mode sélection : cocher les gardés retire le complément", async () => {
  const ctx = ecran();
  ctx.repondreAux(() => null);
  await ctx.app.ouvrirAllegerRun({ num: "RF01", jjs: [5] }, { date: "2099-01-05" });
  // Basculer sur le mode sélection (structure selon maquette : un bouton ou
  // un radio portant « garder »).
  const modeGarder = trouver(ctx.$("modal"),
    (n) => (n.tagName === "button" || n.tagName === "input") && /garder/i.test(String(n.title ?? n.value ?? n.children?.[0] ?? "")));
  assert.ok(modeGarder, "le mode « ne garder que… » doit exister sur un run à venir");
  await modeGarder.click();
  const cases = [];
  trouver(ctx.$("modal"), (n) => { if (n.tagName === "input" && n.type === "checkbox") cases.push(n); return false; });
  assert.equal(cases.length, 3, "une case par compte ACTIF du run");
  cases[0].checked = true; cases[0].dispatchEvent("change");
  assert.match(String(ctx.$("modal").textContent), /1 gardé\(s\) — 2 seront retiré\(s\)/);
  const zone = trouver(ctx.$("modal"), (n) => n.tagName === "textarea");
  zone.value = "réduction du run décidée après incident"; zone.dispatchEvent("input");
  await boutonModale(ctx.$, "Retirer 2").click();
  const appel = ctx.invocations.find(([c]) => c === "plan_retirer");
  assert.deepEqual([...appel[1].cfs].sort(), ["CF2", "CF3"], "le complément des gardés, jamais les retirées");
});

test("run à venir, mode proportionnel : la proposition part au retrait telle quelle", async () => {
  const ctx = ecran();
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_proposer_retrait")
      return ctx.evaluer(`(${JSON.stringify([
        { pa: "Cegedim", retirer: ["CF3"], actifs: 2 },
        { pa: "Esalink", retirer: ["CF2"], actifs: 1 },
      ])})`);
    return null;
  });
  await ctx.app.ouvrirAllegerRun({ num: "RF01", jjs: [5] }, { date: "2099-01-05" });
  const champN = trouver(ctx.$("modal"), (n) => n.tagName === "input" && n.type === "number");
  champN.value = "2"; champN.dispatchEvent("input");
  const proposer = boutonModale(ctx.$, "Proposer");
  await proposer.click();
  assert.ok(ctx.invocations.find(([c]) => c === "plan_proposer_retrait"),
    "la répartition est un calcul métier : elle vient du backend, jamais du JS");
  const zone = trouver(ctx.$("modal"), (n) => n.tagName === "textarea");
  zone.value = "prorata après incident"; zone.dispatchEvent("input");
  await boutonModale(ctx.$, "Retirer 2").click();
  const appel = ctx.invocations.find(([c]) => c === "plan_retirer");
  assert.deepEqual([...appel[1].cfs].sort(), ["CF2", "CF3"]);
});
```

Note d'exécution : vérifier dans `dom_shim.js` comment `ctx.app.renderPlanRecap` est exposé et exposer `ouvrirAllegerRun` par le même canal ; si le shim a des limites (dispatchEvent, `type` des inputs), suivre les motifs des tests existants (`rapprochement.test.js`, `plan_recap.test.js`…) plutôt que d'étendre le shim.

- [ ] **Step 2 : rouge** — `node --test "tests/alleger.test.js"` → échecs (fonction absente).

- [ ] **Step 3 : implémentation** (sous `ouvrirAjoutRun` ; structure DOM selon la maquette validée)

Contrat à respecter, quel que soit le détail maquette :

```js
/** Alléger un run : exclure (passé) / retirer N au prorata / ne garder que…
 *  Les listes de CF viennent du plan chargé (`plan.lignes`) ou du backend
 *  (`plan_proposer_retrait`) — le JS ne calcule jamais une répartition.
 *  UN SEUL `plan_retirer` par geste : le rapport regroupe par (horodatage,
 *  motif), un geste éclaté en plusieurs appels deviendrait plusieurs gestes. */
async function ouvrirAllegerRun(run, jour) {
  const d = new Date();
  const aujourdhui = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const passe = jour.date < aujourdhui;
  const actifs = plan.lignes.filter((l) => l.run_num === run.num && l.retire_motif == null);
  // … modale selon la maquette. Points fixes :
  // - mode exclusion (passé) : motif pré-rempli
  //   `Run ${run.num} du ${dateFr(jour.date)} exclu a posteriori — ` ; le
  //   bouton reste inerte tant que zone.value.trim() n'est pas STRICTEMENT
  //   plus long que le pré-remplissage trimé ; cfs = actifs.map((l) => l.cf).
  // - mode sélection (futur) : cases sur les gardés, pied
  //   `${g} gardé(s) — ${actifs.length - g} seront retiré(s)` recalculé à
  //   chaque change ; cfs = actifs non cochés.
  // - mode proportionnel (futur) : champ N + « Proposer » →
  //   invoke("plan_proposer_retrait", { runNum: run.num, n }) ; affichage
  //   groupé par PA (« ${pa} — ${retirer.length} sur ${actifs} ») ; un
  //   « échanger » remplace un CF proposé par un autre compte actif de la
  //   même PA (liste depuis `actifs`) ; cfs = la proposition amendée.
  // - validation, tous modes : invoke("plan_retirer", { cfs, motif }) puis
  //   plan.sel.clear(); signalerObsoletes(obsoletes); await rechargerRecap();
  //   closeModal(); — le même épilogue qu'`ouvrirRetrait`.
  // - avertissement MEP gelée : reprendre le bloc `danger-note`
  //   d'`ouvrirRetrait` sur les lignes concernées.
}
```

Toute construction DOM via `h()`/`textContent` — les raisons sociales et motifs sont des entrées non fiables.

- [ ] **Step 4 : vert** — `node --test "tests/alleger.test.js"` → 3 pass, puis toute la suite JS.
- [ ] **Step 5 : commit** — `git commit -am "feat(superpopaul): la modale « alléger un run » (exclure / prorata / garder)"`

### Task 9 : bouton « Alléger… » sur la ligne du run (⛔ après le go maquette)

**Files :** Modify `client/src/app.js` (ligne de run de la timeline, l. ~2026).

- [ ] **Step 1 : ajouter le bouton** à côté de « + Ajouter » (libellé selon maquette)

```js
  const ajout = h("td", { class: "tl-add" },
    ...(plan.genere
      ? [h("button", { class: "tl-add-btn", onclick: (ev) =>
            occupe(ev.currentTarget, "…", () => ouvrirAjoutRun(r, j)) }, "+ Ajouter"),
         h("button", { class: "tl-add-btn", onclick: (ev) =>
            occupe(ev.currentTarget, "…", () => ouvrirAllegerRun(r, j)) }, "− Alléger…")]
      : []));
```

Même garde `plan.genere` que l'ajout (les deux retouchent le plan persisté). Un run `ecart` ne porte toujours aucune action (branche l. ~2005 inchangée).

- [ ] **Step 2 : vert** — suite JS complète, puis vérification visuelle dans l'app (timeline : les deux boutons, modale qui s'ouvre sur un run passé et un futur).
- [ ] **Step 3 : commit** — `git commit -am "feat(superpopaul): l'action alléger vit sur la ligne du run"`

### Task 10 : suites complètes + mutations ciblées

- [ ] **Step 1 : tout vert** — `cd client/src-tauri && cargo test` (0 échec — pas de `grep "test result: ok"`, lire le résumé) ; `cd client && node --test "tests/*.test.js"`.

- [ ] **Step 2 : passe de mutation ciblée** — pour chacune, appliquer, vérifier qu'AU MOINS un test échoue, annuler (`git checkout -- <fichier>` restaure depuis l'index : s'assurer que l'arbre est commité avant) :

1. `plan.rs` : `v.len() - 1` → `v.len()` (plancher) → `jamais_le_dernier_compte…` doit rougir.
2. `plan.rs` : supprimer `rang(a).cmp(&rang(b)).then_with(…)` (garder seulement la suite) → `les_protegees_sortent_en_dernier` doit rougir.
3. `commands.rs` : clé de regroupement `(r.le, r.motif.clone())` → `(r.le, String::new())` → `meme_seconde_mais_motifs_differents…` doit rougir.
4. `commands.rs` : `l.mep_date < jour_local(le)` → `l.mep_date < chrono::Local::now().date_naive()` → `un_retrait_anterieur_a_sa_mep_n_est_pas_gele…` doit rougir.
5. `rapprochement_report.rs` : dans la branche multiple de l'alerte, remplacer `fmt_int(*n as u64)` par `fmt_int(g.comptes.len() as u64)` → `l_alerte_agrege_les_geles…` doit rougir (2 ≠ 3).

Si une mutation survit : le test correspondant est incapable d'échouer — le durcir avant de continuer (leçon récurrente des chantiers précédents).

- [ ] **Step 3 : commit éventuel des durcissements** — `git commit -am "test(superpopaul): durcissements issus de la passe de mutation"`

### Task 11 : version 1.9.0 + notes de release (fichier seulement)

- [ ] **Step 1 : bump** — `client/src-tauri/Cargo.toml` (`version = "1.9.0"`), `client/src-tauri/tauri.conf.json` (`"version": "1.9.0"`), puis `cargo check` (met `Cargo.lock` à jour).

- [ ] **Step 2 : notes** — créer `docs/releases/v1.9.0.md`, rédigées pour un humain (le destinataire est l'utilisateur de l'app) : les trois aides à la saisie, le rapport qui regroupe par geste, l'alerte qui ne crie plus pour un retrait antérieur à sa MEP. S'inspirer du ton de `docs/releases/v1.8.0.md`.

- [ ] **Step 3 : commit** — `git commit -am "chore(superpopaul): v1.9.0"`

**Fin de plan.** Ne PAS pousser, ne PAS tagger, ne PAS ouvrir de PR sans demande explicite de l'utilisateur (règle projet : lancer une release et pousser se demandent à chaque fois). La validation GUI est le geste de l'utilisateur — nommer ce qui n'est prouvé que par des tests.
