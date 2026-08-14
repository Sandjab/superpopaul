# Les retraits manuels au rapport de rapprochement — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Faire apparaître dans le rapport de rapprochement les comptes retirés **à la main** depuis la dernière note, pour qu'un lot transmis n'ait plus de comptes disparus sans explication.

**Architecture:** Aucun module nouveau. `rapprochement_report.rs` reçoit deux champs de plus (`retraits_manuels`, `depuis`) et rend une tuile, un tableau et des entrées supplémentaires dans l'alerte rouge. `commands.rs` gagne une fonction **pure et testable** — `retraits_manuels_depuis` — qui déduit « manuel et non encore rapporté » de `retire.le > meta.rapproche_le`. Le lot 2 rend le rapport productible quand il n'y a que des retraits à documenter : une condition et un libellé côté JS, plus un compteur remonté par `plan_rapprocher`.

**Tech Stack:** Rust (Tauri 2, `cargo test` dans `client/src-tauri/`), JS vanilla (`node --test "tests/*.test.js"` depuis `client/`).

**Spec:** `docs/superpowers/specs/2026-08-14-retraits-manuels-au-rapport-design.md`
**Maquettes validées (14/08/2026) :**
- `docs/superpowers/maquettes/2026-08-14-rapport-retraits-manuels.html`
- `docs/superpowers/maquettes/2026-08-14-note-de-livraison.html`

**Convention du dépôt :** commits fréquents, `feat(superpopaul): …` / `test(superpopaul): …` / `fix(superpopaul): …`. Test d'abord pour toute logique Rust. Textes en français. Jamais d'`innerHTML` avec des données dynamiques côté JS.

**Ligne de base mesurée le 14/08 :** suite JS **97 tests, tous verts**. La suite Rust **n'a pas pu être lancée** dans le conteneur de rédaction (les bibliothèques système `gdk-3.0` de Tauri n'y sont pas installées) — relever le compte de départ avec `cargo test` **avant** la tâche 1, et s'y référer ensuite.

**Découpage :** les tâches 1 à 6 forment le **lot 1** (le rapport), les tâches 7 et 8 le **lot 2** (la note de livraison). Les deux sont livrables séparément ; le lot 2 seul n'aurait aucun sens, le lot 1 seul est complet.

---

## Task 1: Le modèle et la cinquième tuile

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs`

- [ ] **Step 1: Write the failing tests**

Dans `mod tests`, **ajouter** aux helpers existants :

```rust
    /// `Depuis` par défaut, partagé : `donnees()` rend une struct empruntante,
    /// elle ne peut pas emprunter un temporaire local. Même motif que
    /// `origines_vides()`.
    fn depuis_defaut() -> &'static Depuis {
        static D: std::sync::OnceLock<Depuis> = std::sync::OnceLock::new();
        D.get_or_init(|| Depuis::DernierRapprochement("2026-07-28".into()))
    }

    fn retrait_manuel(cf: &str, le: &str, motif: &str, gelee: bool) -> RetraitManuel {
        RetraitManuel { cf: cf.into(), le: le.into(), motif: motif.into(), gelee }
    }
```

Et compléter `donnees()` par les deux nouveaux champs :

```rust
            retraits_manuels: &[],
            depuis: depuis_defaut(),
```

Puis les tests :

```rust
    #[test]
    fn une_cinquieme_tuile_compte_les_retraits_manuels() {
        let r = vide();
        let manuels = vec![
            retrait_manuel("4100238091", "2026-07-31", "Périmètre repoussé à 2027", false),
            retrait_manuel("4100243662", "2026-07-31", "Périmètre repoussé à 2027", false),
        ];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("retirés à la main"), "libellé de la tuile absent");
        assert!(c.contains("hors du calcul"), "la tuile doit dire qu'elle est hors du compte");
        assert!(c.contains("<div class=\"v\">2</div>"), "compte des retraits manuels absent");
    }

    #[test]
    fn sans_retrait_manuel_la_cinquieme_tuile_n_existe_pas() {
        // Les quatre autres tuiles décrivent ce que le rapprochement a
        // EXAMINÉ : « 0 déplacé » est un constat. Une tuile à zéro sur des
        // retraits manuels parlerait d'un geste qui n'a pas eu lieu.
        let r = vide();
        let html = render(&donnees(&r));
        assert!(!corps(&html).contains("retirés à la main"));
    }

    #[test]
    fn les_retraits_manuels_ne_faussent_pas_l_arithmetique_du_resume() {
        // `inchangées + retirés + déplacés + rafraîchis + signalés = actives`.
        // Les retraits manuels sont HORS de cette somme — `calculer` les a
        // sautés, ils ne sont ni dans `ecarts` ni dans `inchangees`. Les
        // compter dans « comptes retirés » ferait dépasser le total dans un
        // document transmis.
        let mut r = vide();
        r.inchangees = 143;
        r.ecarts = vec![ecart_eligibilite("4100000001")];
        let manuels = vec![
            retrait_manuel("4100238091", "2026-07-31", "décision métier", false),
            retrait_manuel("4100243662", "2026-07-31", "décision métier", false),
            retrait_manuel("4100247788", "2026-08-06", "litige", false),
        ];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("<div class=\"v\">1</div>"), "un seul retrait CALCULÉ");
        assert!(c.contains("sur <b>144</b> actives"),
            "les actives restent 143 inchangées + 1 écart : les manuels n'y entrent pas");
        assert!(c.contains("<div class=\"v\">3</div>"), "les 3 manuels ont leur propre tuile");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : ÉCHEC à la compilation — `cannot find struct 'RetraitManuel'`, `cannot find type 'Depuis'`, `struct 'RapprochementReportData' has no field named 'retraits_manuels'`.

- [ ] **Step 3: Write minimal implementation**

Au-dessus de `RapprochementReportData`, après `PositionAvant` :

```rust
/// Un compte retiré **à la main**, tel que le rapport en parle.
///
/// Propriétaire de ses chaînes, comme `PositionAvant` et contrairement à
/// `FichierLivre` : la date est CALCULÉE par la commande (conversion d'un
/// horodatage stocké), elle n'existe donc nulle part où l'emprunter.
pub struct RetraitManuel {
    pub cf: String,
    /// Date du retrait, **en ISO** — comme `PositionAvant::run_date` et
    /// `FichierLivre::mep_date`. Le rendu la met en forme via `date_fr`.
    pub le: String,
    /// Saisi par l'utilisateur. **Texte libre**, échappé au point d'insertion.
    pub motif: String,
    /// La MEP de la ligne est passée : le compte figure dans un fichier déjà
    /// transmis. Même conséquence qu'un `Ecart.gelee`, même traitement.
    pub gelee: bool,
}

/// Ce que la liste des retraits manuels prend pour origine. Le document écrit
/// la date ET ce qu'elle désigne : « depuis le 28/07 » ne dit pas au lecteur
/// pourquoi la liste commence là.
pub enum Depuis {
    /// ISO. Le plan a déjà été rapproché : la liste part de cette date-là.
    DernierRapprochement(String),
    /// ISO. Le plan n'a jamais été rapproché : elle part de sa génération.
    GenerationDuPlan(String),
}
```

Dans `RapprochementReportData`, après `origines` :

```rust
    /// Retraits faits à la main depuis `depuis`, **triés par la commande**
    /// (date puis n° de CF) : le rendu ne réordonne pas.
    pub retraits_manuels: &'a [RetraitManuel],
    pub depuis: &'a Depuis,
```

Dans `render`, **juste avant** le `html.push_str("</section>\n");` qui ferme les KPI :

```rust
    // Cinquième tuile, rendue seulement si elle est non nulle — les quatre
    // autres décrivent ce que le rapprochement a examiné, celle-ci un geste
    // qui a eu lieu ou pas. NEUTRE : elle est hors de l'arithmétique des
    // autres, lui donner du rouge en ferait un second total de retraits.
    if !d.retraits_manuels.is_empty() {
        html.push_str(&format!(
            "<div class=\"kpi hors\"><div class=\"v\">{}</div>\
             <div class=\"l\">retirés à la main</div>\
             <div class=\"abs\">hors du calcul</div></div>\n",
            fmt_int(d.retraits_manuels.len() as u64),
        ));
    }
```

Et en fin de `CSS_RAPPRO`, avant le `"#` de clôture :

```
  .kpi.hors { border-left: 3px solid var(--pa-autres); }
  .kpi.hors .v { color: var(--fg); }
  td.date { white-space: nowrap; color: var(--muted); }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : les 3 nouveaux passent, les 16 existants restent verts.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs
git commit -m "feat(superpopaul): une tuile pour les retraits manuels du rapport"
```

---

## Task 2: Le tableau « Comptes retirés — décision manuelle »

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs`

- [ ] **Step 1: Write the failing tests**

```rust
    const T_MANUELS: &str = "<h2>Comptes retirés — décision manuelle</h2>";

    #[test]
    fn le_tableau_des_retraits_manuels_donne_date_et_motif() {
        let r = vide();
        let manuels = vec![
            retrait_manuel("4100238091", "2026-07-31",
                "Exclusion décidée en comité — périmètre repoussé à 2027", false),
            retrait_manuel("4100247788", "2026-08-06",
                "Litige commercial en cours", true),
        ];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains(T_MANUELS), "section des retraits manuels absente");
        assert!(c.contains("4100238091"));
        // La date est rendue en clair : elle est ce qui distingue ce tableau
        // des deux autres, dont les comptes sont retirés à l'instant.
        assert!(c.contains("31/07/2026"), "date du retrait absente ou non mise en forme");
        assert!(c.contains("06/08/2026"));
        assert!(c.contains("Exclusion décidée en comité"), "motif absent");
    }

    #[test]
    fn le_tableau_des_retraits_manuels_dit_depuis_quand() {
        // La date de référence n'est pas décorative : sans elle, le lecteur ne
        // sait pas si la liste couvre une semaine ou six mois.
        let r = vide();
        let manuels = vec![retrait_manuel("4100238091", "2026-07-31", "décision métier", false)];

        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let apres_rappro = Depuis::DernierRapprochement("2026-07-28".into());
        d.depuis = &apres_rappro;
        let c1 = render(&d).to_string();
        assert!(corps(&c1).contains("28/07/2026"), "date de référence absente");
        assert!(corps(&c1).contains("dernier rapprochement"),
            "la nature de la date de référence doit se lire");

        let jamais = Depuis::GenerationDuPlan("2026-06-02".into());
        d.depuis = &jamais;
        let c2 = render(&d).to_string();
        assert!(corps(&c2).contains("02/06/2026"));
        assert!(corps(&c2).contains("génération du plan"),
            "un plan jamais rapproché doit le dire, pas inventer un rapprochement");
        assert!(!corps(&c2).contains("dernier rapprochement"),
            "les deux formulations ne doivent jamais coexister");
    }

    #[test]
    fn sans_retrait_manuel_le_tableau_n_existe_pas() {
        let r = vide();
        let html = render(&donnees(&r));
        assert!(!corps(&html).contains(T_MANUELS));
    }

    #[test]
    fn un_motif_saisi_par_l_utilisateur_sort_echappe() {
        // Première chaîne d'origine HUMAINE du document : les motifs des deux
        // autres tableaux sont générés par le code (`format!`), celui-ci est
        // tapé dans une boîte de dialogue. Un `esc` oublié ici injecte du
        // balisage dans une pièce transmise.
        let r = vide();
        let manuels = vec![retrait_manuel(
            "<script>alert(1)</script>", "2026-07-31", "A&B <script>alert(2)</script>", false)];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(!c.contains("<script>"), "motif ou n° de CF non échappé");
        assert!(c.contains("&lt;script&gt;"));
        assert!(c.contains("A&amp;B"), "l'esperluette du motif doit être échappée");
        assert!(!c.contains("&amp;amp;"), "échappé deux fois");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : 4 échecs — la section n'existe pas.

- [ ] **Step 3: Write the implementation**

Dans `render`, **entre** la fin de la section ② (« disparus du fichier », son `fin_section`) et le début de la section des déplacés :

```rust
    // ③ Retirés à la main. Groupé avec ① et ② : le document range par
    // CONSÉQUENCE pour le destinataire, et les trois répondent à « quels
    // comptes ne sont plus dans mes fichiers ». La colonne « Retiré le » est
    // ce qui distingue ce tableau — ses comptes ont pu quitter le plan des
    // semaines avant ce rapprochement.
    let sous_titre_manuels = match d.depuis {
        Depuis::DernierRapprochement(iso) => format!(
            "Retirés à la main depuis le {}, dernier rapprochement. \
             Ces comptes ne figurent dans aucun fichier de ce lot.",
            date_fr(iso)
        ),
        Depuis::GenerationDuPlan(iso) => format!(
            "Retirés à la main depuis la génération du plan, le {}. \
             Ces comptes ne figurent dans aucun fichier de ce lot.",
            date_fr(iso)
        ),
    };
    section(
        &mut html,
        "Comptes retirés — décision manuelle",
        &sous_titre_manuels,
        &["N° de CF", "Retiré le", "Motif"],
        d.retraits_manuels.is_empty(),
    );
    for m in d.retraits_manuels {
        html.push_str(&format!(
            "<tr><td>{}</td><td class=\"date\">{}</td><td>{}</td></tr>\n",
            esc(&m.cf),
            esc(&date_fr(&m.le)),
            esc(&m.motif),
        ));
    }
    fin_section(&mut html, d.retraits_manuels.is_empty());
```

> Renuméroter les commentaires des sections suivantes : les déplacés passent de ③ à ④, les plateformes de ④ à ⑤.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : tout vert.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs
git commit -m "feat(superpopaul): le tableau des retraits manuels du rapport"
```

---

## Task 3: Les retraits manuels gelés rejoignent l'alerte rouge

La section rouge existe pour une **conséquence**, pas pour une provenance : les `.txt` étant cumulatifs, le destinataire tient une version antérieure où le compte figurait. Un retrait manuel sur une MEP passée produit exactement cette situation.

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn un_retrait_manuel_gele_rejoint_l_alerte_des_mep_transmises() {
        let r = vide();
        let manuels = vec![
            retrait_manuel("4100247788", "2026-08-06",
                "Litige commercial en cours — ne pas facturer", true),
            retrait_manuel("4100238091", "2026-07-31", "périmètre 2027", false),
        ];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains(T_ALERTE), "section d'alerte absente");
        // Dans l'alerte ET dans le tableau : la mise en évidence ne dispense
        // pas du tableau — c'est déjà le sort des retraits calculés gelés.
        let alerte = c.split(T_ALERTE).nth(1).expect("alerte absente")
            .split("</section>").next().unwrap_or("");
        assert!(alerte.contains("4100247788"), "le gelé doit être dans l'alerte");
        assert!(alerte.contains("Litige commercial en cours"),
            "le motif de l'utilisateur dit la décision mieux qu'un libellé généré");
        assert!(!alerte.contains("4100238091"),
            "un retrait manuel NON gelé n'a rien à faire dans l'alerte");
        assert!(c.contains(T_MANUELS), "le gelé doit AUSSI figurer au tableau");
    }

    #[test]
    fn sans_retrait_gele_ni_calcule_ni_manuel_l_alerte_n_existe_pas() {
        let r = vide();
        let manuels = vec![retrait_manuel("4100238091", "2026-07-31", "périmètre 2027", false)];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        assert!(!corps(&html).contains(T_ALERTE));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : le premier échoue (le gelé manuel n'est pas dans l'alerte), le second passe déjà — non-régression, il doit le rester.

- [ ] **Step 3: Write the implementation**

Remplacer l'ouverture du bloc `geles` par :

```rust
    let geles: Vec<&&Ecart> = retires.iter().filter(|e| e.gelee).collect();
    // Même conséquence, même section : le destinataire tient une version
    // antérieure du fichier, que le retrait vienne du calcul ou d'une
    // décision. Les manuels viennent à la suite — ordre déterministe.
    let geles_manuels: Vec<&RetraitManuel> =
        d.retraits_manuels.iter().filter(|m| m.gelee).collect();
    if !geles.is_empty() || !geles_manuels.is_empty() {
```

et, juste avant le `html.push_str("</ul>\n</section>\n");` qui ferme ce bloc, ajouter la seconde boucle :

```rust
        for m in &geles_manuels {
            html.push_str(&format!(
                "<li>Le compte <b>{}</b> figurait dans un fichier qui vous a déjà été \
                 transmis. Les fichiers étant cumulatifs, il ne figure plus dans aucun \
                 fichier de ce lot. Motif : <b>{}</b>.</li>\n",
                esc(&m.cf),
                esc(&m.motif),
            ));
        }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs
git commit -m "feat(superpopaul): un retrait manuel gelé rejoint l'alerte des MEP transmises"
```

---

## Task 4: `retraits_manuels_depuis` — le filigrane

Le cœur du chantier, et son seul point fragile. Fonction **pure**, testable sans `tauri::State`, comme `fichiers_obsoletes` ou `avertissement_ppf_cumulatif`.

**Files:**
- Modify: `client/src-tauri/src/commands.rs` (deux fonctions libres + module `tests`)

- [ ] **Step 1: Write the failing tests**

Dans le module `tests` de `commands.rs`. Le helper `ligne_plan(cf, mep_id, mep_date)` existe déjà (ajouté au chantier du 28/07) ; lui adjoindre :

```rust
    fn ligne_retiree(cf: &str, mep_date: &str, le: i64, motif: &str) -> crate::plan::LignePlan {
        let mut l = ligne_plan(cf, 1, mep_date);
        l.retire = Some(crate::plan::Retrait { le, motif: motif.into() });
        l
    }

    /// Le 6 août 2026 à midi UTC. Choisi à midi : la date locale est alors la
    /// même de UTC-12 à UTC+11, donc l'assertion ne dépend pas du fuseau de la
    /// machine de test.
    const LE_6_AOUT: i64 = 1_786_017_600;
    const LE_31_JUILLET: i64 = 1_785_499_200;
    const LE_28_JUILLET: i64 = 1_785_240_000;
```

> Vérifier ces trois constantes avant de les figer :
> `date -u -d @1786017600` doit rendre `Thu Aug  6 12:00:00 UTC 2026`. Ajuster
> si le calcul diffère — c'est la **valeur** qui compte, pas le littéral.

```rust
    #[test]
    fn un_retrait_pose_apres_le_dernier_rapprochement_est_liste() {
        let lignes = vec![ligne_retiree("4100238091", "2026-12-01", LE_31_JUILLET, "périmètre 2027")];
        let out = retraits_manuels_depuis(&lignes, Some(LE_28_JUILLET), jour("2026-08-14"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].cf, "4100238091");
        assert_eq!(out[0].le, "2026-07-31", "la date doit être rendue en ISO");
        assert_eq!(out[0].motif, "périmètre 2027");
        assert!(!out[0].gelee, "la MEP du 01/12 n'est pas passée le 14/08");
    }

    #[test]
    fn un_retrait_pose_par_le_rapprochement_lui_meme_n_est_pas_liste() {
        // VERROU DU FILIGRANE. `plan_rapprocher_appliquer` calcule `maintenant`
        // UNE SEULE FOIS et le pose sur les retraits qu'il crée COMME sur
        // `meta.rapproche_le` : les deux valeurs sont donc égales, et la
        // comparaison stricte les exclut. Si l'horloge venait à être lue deux
        // fois, avec `rapproche_le` calculé AVANT `appliquer`, les retraits du
        // rapprochement n resurgiraient dans le rapport n+1 sous l'étiquette
        // « manuel ». Ce test fige la borne ; le commentaire de la commande
        // fige la cause.
        let lignes = vec![ligne_retiree(
            "4100241902", "2026-12-01", LE_28_JUILLET,
            "Rapprochement du 28/07/2026 — CTC prêt plus tard")];
        let out = retraits_manuels_depuis(&lignes, Some(LE_28_JUILLET), jour("2026-08-14"));
        assert!(out.is_empty(), "un retrait du rapprochement n'est pas un retrait manuel");
    }

    #[test]
    fn un_retrait_anterieur_au_dernier_rapprochement_n_est_pas_liste() {
        // Il a déjà été documenté par la note précédente : le lister à nouveau
        // le ferait apparaître dans deux rapports.
        let lignes = vec![ligne_retiree("4100238091", "2026-12-01", LE_28_JUILLET - 3600, "vieux")];
        let out = retraits_manuels_depuis(&lignes, Some(LE_28_JUILLET), jour("2026-08-14"));
        assert!(out.is_empty());
    }

    #[test]
    fn sans_rapprochement_anterieur_tous_les_retraits_sont_manuels() {
        // `rapproche_le` à None : seul `plan::retirer` a pu poser ces retraits.
        let lignes = vec![ligne_retiree("4100238091", "2026-12-01", LE_31_JUILLET, "périmètre 2027")];
        let out = retraits_manuels_depuis(&lignes, None, jour("2026-08-14"));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn une_ligne_active_ne_produit_aucun_retrait() {
        let lignes = vec![ligne_plan("4100240115", 1, "2026-12-01")];
        let out = retraits_manuels_depuis(&lignes, None, jour("2026-08-14"));
        assert!(out.is_empty());
    }

    #[test]
    fn la_liste_est_ordonnee_par_date_puis_par_compte() {
        // Une liste de décisions se lit comme un journal ; un retrait en lot
        // pose la MÊME seconde sur toutes ses lignes, que le n° de CF départage.
        // Les deux clés sont exercées : sans la seconde, l'ordre des deux
        // lignes du 31/07 dépendrait de celui du plan.
        let lignes = vec![
            ligne_retiree("4100247788", "2026-12-01", LE_6_AOUT, "litige"),
            ligne_retiree("4100243662", "2026-12-01", LE_31_JUILLET, "comité"),
            ligne_retiree("4100238091", "2026-12-01", LE_31_JUILLET, "comité"),
        ];
        let out = retraits_manuels_depuis(&lignes, None, jour("2026-08-14"));
        let cfs: Vec<&str> = out.iter().map(|m| m.cf.as_str()).collect();
        assert_eq!(cfs, vec!["4100238091", "4100243662", "4100247788"]);
    }

    #[test]
    fn un_retrait_sur_une_mep_passee_est_marque_gele() {
        // C'est ce drapeau qui envoie la ligne dans l'alerte rouge du rapport.
        let lignes = vec![ligne_retiree("4100247788", "2026-06-12", LE_6_AOUT, "litige")];
        let out = retraits_manuels_depuis(&lignes, None, jour("2026-08-14"));
        assert!(out[0].gelee, "la MEP du 12/06 est passée le 14/08");
    }
```

> `jour(...)` : si le module `tests` de `commands.rs` n'a pas déjà ce helper,
> ajouter `fn jour(iso: &str) -> chrono::NaiveDate { chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap() }`.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test retraits_manuels
```

Attendu : ÉCHEC à la compilation — `cannot find function 'retraits_manuels_depuis'`.

- [ ] **Step 3: Write the implementation**

À côté de `avertissement_ppf_cumulatif` (`commands.rs:1790`) :

```rust
/// Un horodatage stocké rendu en date ISO du **fuseau local**.
///
/// Les horodatages de retrait sont posés en UTC (`Utc::now().timestamp()`),
/// mais le document est lu par celui qui a fait le geste, à son heure. Passer
/// par `Utc` avant de convertir évite l'ambiguïté d'un `Local.timestamp_opt`
/// sur un changement d'heure — un instant UTC n'est jamais ambigu.
fn jour_local_iso(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.with_timezone(&chrono::Local).date_naive().to_string())
        .unwrap_or_default()
}

/// Les retraits faits **à la main** que la dernière note n'a pas documentés.
///
/// Rien ne marque l'origine d'un retrait — `Retrait` porte une date et un
/// motif, pas sa provenance. Elle se DÉDUIT de deux horodatages :
/// `plan_rapprocher_appliquer` calcule `maintenant` une seule fois et s'en
/// sert pour les retraits qu'il pose ET pour `meta.rapproche_le`. Les retraits
/// d'un rapprochement portent donc exactement la date de rapprochement
/// enregistrée, et la comparaison **stricte** les exclut à la seconde près.
///
/// `rapproche_le` à `None` : le plan n'a jamais été rapproché, donc tous les
/// retraits présents sont manuels — seul `plan::retirer` a pu les poser.
///
/// Collision acceptée : un retrait manuel posé dans la même seconde qu'une
/// application de rapprochement est classé avec ceux du rapprochement, donc
/// jamais listé. Fenêtre d'une seconde sur une application de bureau
/// mono-utilisateur ; la fermer demanderait une colonne d'origine en base,
/// écartée à la conception.
fn retraits_manuels_depuis(
    lignes: &[crate::plan::LignePlan],
    rapproche_le: Option<i64>,
    aujourdhui: chrono::NaiveDate,
) -> Vec<crate::rapprochement_report::RetraitManuel> {
    let seuil = rapproche_le.unwrap_or(i64::MIN);
    let mut out: Vec<crate::rapprochement_report::RetraitManuel> = lignes
        .iter()
        .filter_map(|l| {
            let r = l.retire.as_ref()?;
            (r.le > seuil).then(|| crate::rapprochement_report::RetraitManuel {
                cf: l.cf.clone(),
                le: jour_local_iso(r.le),
                motif: r.motif.clone(),
                gelee: l.gelee(aujourdhui),
            })
        })
        .collect();
    // Date puis compte : les dates ISO se comparent lexicographiquement, et un
    // retrait en lot pose la même seconde sur toutes ses lignes.
    out.sort_by(|a, b| a.le.cmp(&b.le).then_with(|| a.cf.cmp(&b.cf)));
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test retraits_manuels
```

Attendu : `7 passed`.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): déduire les retraits manuels non documentés"
```

---

## Task 5: `plan_rapprocher_appliquer` alimente le rapport

**Files:**
- Modify: `client/src-tauri/src/commands.rs` (`plan_rapprocher_appliquer`, ~ligne 1878)

- [ ] **Step 1: Brancher la capture**

Juste après la capture de `fichier_avant` et **avant** `rapprochement::appliquer` :

```rust
        // Capturés AVANT le réalignement de `meta` plus bas, qui écrase
        // `rapproche_le` : c'est LUI qui borne « pas encore documenté ». Les
        // lire après ferait une liste systématiquement vide.
        let aujourdhui = chrono::Local::now().date_naive();
        let retraits_manuels = retraits_manuels_depuis(&lignes, meta.rapproche_le, aujourdhui);
        let depuis = match meta.rapproche_le {
            Some(t) => crate::rapprochement_report::Depuis::DernierRapprochement(jour_local_iso(t)),
            None => {
                crate::rapprochement_report::Depuis::GenerationDuPlan(jour_local_iso(meta.genere_le))
            }
        };
```

> `lignes` est lu **avant** `appliquer`. Celle-ci ne touche jamais une ligne
> déjà retirée — les écarts ne peuvent pas en désigner, `calculer` les ayant
> sautées (`rapprochement.rs:94`) — mais capturer au même endroit que
> `fichier_avant` et `origines` évite d'avoir à redémontrer cette étanchéité.

Et **compléter** le commentaire qui accompagne `let maintenant = …` :

```rust
        // UNE SEULE lecture de l'horloge, posée à la fois sur les retraits que
        // le rapprochement crée et sur `meta.rapproche_le` ci-dessous. C'est
        // cette égalité qui permet à `retraits_manuels_depuis` de distinguer un
        // retrait manuel d'un retrait calculé. Lire l'horloge deux fois, avec
        // `rapproche_le` calculé AVANT `appliquer`, ferait réapparaître les
        // retraits de CE rapprochement dans le rapport du suivant.
        let maintenant = chrono::Utc::now().timestamp();
```

- [ ] **Step 2: Passer les deux champs au rendu**

Dans la construction de `RapprochementReportData`, après `origines` :

```rust
                retraits_manuels: &retraits_manuels,
                depuis: &depuis,
```

- [ ] **Step 3: Compiler et lancer la suite**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
```

Attendu : compile, aucune régression. ⚠️ Lire la sortie entière, pas seulement la présence de `test result: ok` — une suite rouge peut afficher cette ligne pour d'autres binaires de test.

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): le rapport de rapprochement porte les retraits manuels"
```

---

## Task 6: Passe de mutation

Sur ce projet, chaque tâche Rust a livré au moins un test **incapable d'échouer**. Le but n'est pas de trouver des tests manquants, mais des tests qui ne prouvent rien.

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs`, `client/src-tauri/src/commands.rs` (tests durcis)

- [ ] **Step 1: Appliquer les mutations une à une**

Pour chacune : modifier le code, lancer la suite, **noter si elle reste verte**, puis annuler.

| # | Mutation | Doit faire rougir |
|---|---|---|
| 1 | `retraits_manuels_depuis` : `>` devient `>=` | `un_retrait_pose_par_le_rapprochement_lui_meme_n_est_pas_liste` |
| 2 | `retraits_manuels_depuis` : `>` devient `<` | les trois tests de sélection |
| 3 | `unwrap_or(i64::MIN)` devient `unwrap_or(i64::MAX)` | `sans_rapprochement_anterieur_tous_les_retraits_sont_manuels` |
| 4 | Le `sort_by` est retiré | `la_liste_est_ordonnee_par_date_puis_par_compte` |
| 5 | Le `then_with` du tri est retiré | idem — si non, le test n'exerce pas la seconde clé |
| 6 | `gelee` est câblé à `false` | `un_retrait_sur_une_mep_passee_est_marque_gele` **et** le test de l'alerte |
| 7 | `gelee` est câblé à `true` | `un_retrait_manuel_gele_rejoint_l_alerte_des_mep_transmises` |
| 8 | La tuile est rendue même à zéro | `sans_retrait_manuel_la_cinquieme_tuile_n_existe_pas` |
| 9 | La tuile compte `ecarts.len()` au lieu des manuels | `une_cinquieme_tuile_compte_les_retraits_manuels` |
| 10 | Les manuels sont ajoutés à `retires.len()` | `les_retraits_manuels_ne_faussent_pas_l_arithmetique_du_resume` |
| 11 | `Depuis` : les deux bras rendent la même phrase | `le_tableau_des_retraits_manuels_dit_depuis_quand` |
| 12 | `date_fr` n'est plus appliquée à `m.le` | `le_tableau_des_retraits_manuels_donne_date_et_motif` |
| 13 | `esc` retiré sur `m.motif` | `un_motif_saisi_par_l_utilisateur_sort_echappe` |
| 14 | `esc` retiré sur `m.cf` | idem |
| 15 | Le filtre `m.gelee` de l'alerte est retiré | `un_retrait_manuel_gele_rejoint_l_alerte_des_mep_transmises` |
| 16 | La boucle des manuels gelés est retirée de l'alerte | idem |
| 17 | La condition d'ouverture de l'alerte redevient `!geles.is_empty()` | idem |

- [ ] **Step 2: Combler chaque mutation survivante**

Une mutation qui laisse la suite verte est un test à écrire ou à durcir. Une mutation peut être **équivalente** (HTML identique) — le noter en commentaire plutôt que d'inventer un test.

- [ ] **Step 3: Lancer les deux suites**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
cd ../ && node --test "tests/*.test.js"
```

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs client/src-tauri/src/commands.rs
git commit -m "test(superpopaul): durcit les retraits manuels contre les mutations"
```

---

> **Fin du lot 1.** Le rapport est complet et livrable en l'état. Les tâches 7 et 8 forment le lot 2 et peuvent être reportées.

---

## Task 7: `plan_rapprocher` remonte le compte des retraits à documenter

**Files:**
- Modify: `client/src-tauri/src/commands.rs` (`RapprochementVue`, `plan_rapprocher`)

- [ ] **Step 1: Ajouter le champ**

Dans `RapprochementVue` :

```rust
    /// Retraits faits à la main que ce rapprochement documentera. Un **compte**
    /// et non une liste : l'écran n'en affiche pas le détail — celui qui
    /// applique vient de les faire — mais ce nombre décide si l'application a
    /// quelque chose à écrire quand il n'y a aucun écart.
    pub retraits_manuels: usize,
```

- [ ] **Step 2: Le calculer dans `plan_rapprocher`**

Les deux valeurs jetées par `let (rapprochement, empreinte, _, _, annuaire_incomplet)` servent maintenant :

```rust
        let (rapprochement, empreinte, lignes, meta, annuaire_incomplet) =
            calculer_rapprochement(&store, &input, &cfg)?;
        let aujourdhui = chrono::Local::now().date_naive();
        let retraits_manuels =
            retraits_manuels_depuis(&lignes, meta.rapproche_le, aujourdhui).len();
        Ok(RapprochementVue { rapprochement, empreinte, annuaire_incomplet, retraits_manuels })
```

- [ ] **Step 3: Compiler et lancer la suite Rust**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
```

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): le calcul du rapprochement compte les retraits à documenter"
```

---

## Task 8: La modale « sans écart » produit la note de livraison

⚠️ **Le compilateur Rust ne voit pas les doublures JS.** Une doublure qui ne rend pas le nouveau champ ferait porter au code un état impossible.

**Files:**
- Modify: `client/src/app.js` (`ouvrirRapprocher` ~2738, `compteRenduRapprochement` ~2650)
- Modify: `client/tests/rapprochement.test.js`

- [ ] **Step 1: Vérifier l'étendue des doublures**

```bash
cd client && grep -rn "plan_rapprocher\b" tests/ src/
```

Traiter tout ce qui apparaît, pas seulement les emplacements listés ici.

- [ ] **Step 2: Write the failing tests**

Dans `client/tests/rapprochement.test.js` :

```js
test("sans écart mais avec des retraits manuels, l'application produit la note", async () => {
  // Cas réel : on retire des comptes à la main, on relance une résolution qui
  // ne fait basculer aucun verdict, on rapproche. Zéro écart — et pourtant le
  // lot partirait sans note alors que trois comptes ont quitté les fichiers.
  const ctx = ecran();
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [], inchangees: 2001, avertissements: [] },
        empreinte: "peu importe ici",
        annuaire_incomplet: null,
        retraits_manuels: 3,
      })})`);
    if (cmd === "plan_lignes") return ctx.evaluer(`(${JSON.stringify([ligne("CF1")])})`);
    if (cmd === "plan_rapprocher_appliquer")
      return ctx.evaluer(`(${JSON.stringify({
        obsoletes: [], rapport: "/sortie/brm2608_rapprochement_2026-08-14_143207.html",
      })})`);
    return null;
  });

  await boutonRapprocher(ctx.$).click();

  const produire = boutonModale(ctx.$, "Produire la note");
  assert.ok(produire, "le déclencheur doit changer de libellé, pas rester « Appliquer »");
  assert.equal(produire.disabled, false, "il y a quelque chose à écrire");
  await produire.click();

  assert.ok(ctx.invocations.find(([c]) => c === "plan_rapprocher_appliquer"),
    "l'application doit partir vers le backend");
  const texte = String(ctx.$("plan-banner").children?.[0] ?? "");
  assert.match(texte, /3 retrait/, "le compte rendu doit dire ce qui a été documenté");
  assert.match(texte, /brm2608_rapprochement_2026-08-14_143207\.html/,
    "le bandeau nomme le rapport : c'est la seule trace du livrable");
});
```

Et **compléter** la doublure du test existant « sans écart, le déclencheur d'application reste inerte » par `retraits_manuels: 0` — il devient le témoin du cas où rien n'est à écrire, et doit rester vert **sans autre changement** : le libellé reste « Appliquer », le bouton reste inerte.

- [ ] **Step 3: Run tests to verify they fail**

```bash
cd client && node --test "tests/*.test.js"
```

Attendu : le nouveau test échoue (aucun bouton « Produire la note »), le témoin reste vert.

- [ ] **Step 4: Write the implementation**

Dans `ouvrirRapprocher`, remplacer la branche sans écart :

```js
  const { rapprochement, empreinte, annuaire_incomplet: annuaireIncomplet,
          retraits_manuels: retraitsManuels = 0 } = vue;
  if (!rapprochement.ecarts.length) {
    renderSansEcart(rapprochement, empreinte, retraitsManuels);
    return;
  }
```

et ajouter, à côté de `renderRevueRapprochement` :

```js
/** Aucun écart. Deux situations, un seul écran : soit il n'y a rien à écrire —
 *  déclencheur inerte, comme avant —, soit des retraits faits à la main
 *  attendent d'être documentés et l'application a un livrable à produire, sans
 *  toucher à un seul compte. Le libellé s'adapte à ce sur quoi il agit, comme
 *  « Réactiver n retiré(s)… ». */
function renderSansEcart(rapprochement, empreinte, retraitsManuels) {
  const aEcrire = retraitsManuels > 0;
  const bouton = h("button", aEcrire ? { class: "btn-primary", onclick: (ev) =>
    occupe(ev.currentTarget, "Production en cours…", async () => {
      try {
        const { obsoletes, rapport } = await invoke("plan_rapprocher_appliquer", { empreinte });
        closeModal();
        plan.rapportFichier = "identique"; // le backend vient d'aligner meta.hash dessus
        await rechargerRecap();
        compteRenduRapprochement(rapprochement, obsoletes, rapport, retraitsManuels);
      } catch (e) {
        closeModal();
        planBanner("error", String(e),
          h("button", { class: "btn-primary",
            onclick: (ev2) => ouvrirRapprocher(ev2.currentTarget) }, "Rapprocher…"));
      }
    }) } : {},
    aEcrire ? "Produire la note de livraison" : "Appliquer");
  if (!aEcrire) bouton.disabled = true;

  const corps = [h("p", {}, `✓ ${aEcrire ? "Aucun écart avec le fichier ouvert." : "Le plan est à jour avec le fichier ouvert."} `
    + `${fmtN(rapprochement.inchangees)} ligne(s) active(s)${aEcrire ? "." : ", aucun écart."}`)];
  if (aEcrire)
    corps.push(h("div", { class: "rappro-avert" },
      h("b", {}, `${fmtN(retraitsManuels)} retrait(s) fait(s) à la main ne figurent dans aucune note transmise.`),
      " Les comptes concernés ont déjà quitté les fichiers de MEP ; ce qui manque, c'est le document qui l'explique au destinataire."),
      h("p", { class: "rappro-recap" },
        "Aucun compte ne bouge : la note est écrite, les fichiers de MEP sont réécrits à l'identique et le plan se réaligne sur le fichier ouvert."));

  modal(
    h("h3", {}, "Rapprochement du plan"),
    ...corps,
    h("div", { class: "add-foot" },
      h("span", { class: "spacer" }),
      bouton,
      h("button", { class: "btn-ghost", onclick: closeModal }, "Fermer")));
}
```

> `rappro-avert` et `rappro-recap` existent déjà dans `styles.css` (écran de
> revue). Si `rappro-avert` n'y est pas, réutiliser la classe employée par
> `blocAvertissementsRapprochement` plutôt que d'en créer une.

Et `compteRenduRapprochement` gagne un quatrième paramètre :

```js
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
  let texte = parts.length
    ? `✓ Rapprochement appliqué : ${parts.join(", ")}.`
    : (retraitsManuels
        ? `✓ Note de livraison produite : ${fmtN(retraitsManuels)} retrait(s) manuel(s) documenté(s), aucun compte modifié.`
        : "✓ Rapprochement appliqué.");
  if (parts.length && retraitsManuels)
    texte += ` ${fmtN(retraitsManuels)} retrait(s) manuel(s) documenté(s).`;
  const noms = (obsoletes ?? []).map((c) => c.split(/[/\\]/).pop());
  if (noms.length) texte += ` ${noms.length} fichier(s) obsolète(s) supprimé(s) : ${noms.join(", ")}.`;
  if (rapport) texte += ` Rapport : ${rapport.split(/[/\\]/).pop()}.`;
  planBanner("ok", texte);
}
```

Enfin, dans `renderRevueRapprochement`, passer le compte au compte rendu — il faut donc que `retraitsManuels` y parvienne. Ajouter le paramètre à la signature et à l'appel de `ouvrirRapprocher`, sur le modèle de `annuaireIncomplet`.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd client && node --test "tests/*.test.js"
```

Attendu : tout vert, **98 tests ou plus**.

- [ ] **Step 6: Commit**

```bash
git add client/src/app.js client/tests/rapprochement.test.js
git commit -m "feat(superpopaul): produire la note de livraison sans écart"
```

---

## Task 9: Vérification de bout en bout

- [ ] **Step 1: Les deux suites**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
cd ../ && node --test "tests/*.test.js"
```

Attendu : tout vert, et un compte de tests Rust supérieur à celui relevé avant la tâche 1.

- [ ] **Step 2: Comparer le rendu à la maquette**

Produire un rapport de démonstration et l'ouvrir à côté de
`docs/superpowers/maquettes/2026-08-14-rapport-retraits-manuels.html`. Vérifier :
la cinquième tuile est neutre et non rouge ; le tableau « décision manuelle »
est bien **entre** les disparus et les déplacés ; le gelé manuel apparaît dans
l'alerte rouge **et** dans le tableau ; la ligne de sous-titre nomme la date de
référence et sa nature.

- [ ] **Step 3: Ce qui ne se prouve qu'en application**

À signaler à l'utilisateur pour son parcours GUI — `tauri::State` n'est pas
constructible hors application montée :

- retirer deux comptes à la main, rapprocher, appliquer : les deux figurent au
  rapport avec leur motif et leur date ;
- **rapprocher et appliquer une seconde fois** : les mêmes retraits n'y sont
  **plus** — c'est le filigrane à l'œuvre, et l'unique vérification de bout en
  bout que `maintenant` est bien posé une seule fois ;
- retirer un compte d'une MEP passée : il apparaît dans l'alerte rouge ;
- sur un plan jamais rapproché, le sous-titre dit « depuis la génération du
  plan » ;
- lot 2 : sans aucun écart ni retrait manuel, le bouton reste « Appliquer » et
  inerte ; avec des retraits, il devient « Produire la note de livraison » et le
  bandeau nomme le rapport.

- [ ] **Step 4: Ne pas pousser**

Le push et la release restent demandés à chaque fois. S'arrêter ici et rendre compte.
