# Rapport de rapprochement — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produire, à chaque application d'un rapprochement, un rapport HTML horodaté qui accompagne les fichiers de livraison et dit ce qui a changé.

**Architecture:** Un module pur `rapprochement_report.rs` calqué sur `plan_report.rs` (données en entrée, HTML en sortie, aucune I/O ni horloge), alimenté par `plan_rapprocher_appliquer` une fois les livrables écrits. `sauver_apres_retouche` remonte désormais les fichiers de MEP qu'il a produits, information qu'il jetait.

**Tech Stack:** Rust (Tauri 2, `cargo test` dans `client/src-tauri/`), JS vanilla (`node --test "tests/*.test.js"` depuis `client/`).

**Spec:** `docs/superpowers/specs/2026-07-28-rapport-rapprochement-design.md`
**Maquette validée:** `docs/superpowers/maquettes/2026-07-28-rapport-rapprochement.html`

**Convention du dépôt :** commits fréquents, `feat(superpopaul): …` / `test(superpopaul): …` / `fix(superpopaul): …`. Test d'abord pour toute logique Rust. Textes en français. Jamais d'`innerHTML` avec des données dynamiques côté JS.

---

## Task 1: `FichierMep` porte la date de MEP

`ecrire_fichiers_mep` construit le nom du fichier avec la date, mais ne la rend pas : `FichierMep` porte `chemin`, `mep_id`, `comptes`. Le rapport en a besoin, et la reparser depuis le nom ferait du nom un format à maintenir.

**Files:**
- Modify: `client/src-tauri/src/commands.rs:1103-1107` (struct `FichierMep`)
- Modify: `client/src-tauri/src/commands.rs:1161-1180` (boucle d'écriture)
- Test: `client/src-tauri/src/commands.rs` (module `tests` en fin de fichier)

- [ ] **Step 1: Write the failing test**

Ajouter dans le module `tests` de `commands.rs` :

```rust
#[test]
fn un_fichier_de_mep_porte_la_date_qui_figure_dans_son_nom() {
    // La date du nom et la date remontée viennent de la même source : si
    // elles divergent, le rapport annonce une MEP qui n'est pas celle du
    // fichier que le destinataire a en main.
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("brm2607.csv");
    let lignes = vec![
        ligne_plan("4100000001", 1, "2026-05-15"),
        ligne_plan("4100000002", 2, "2026-06-12"),
    ];

    let (ecrits, _) = ecrire_fichiers_mep(&input, ".", &lignes).unwrap();

    assert_eq!(ecrits.len(), 2);
    assert_eq!(ecrits[0].mep_date, "2026-05-15");
    assert_eq!(ecrits[1].mep_date, "2026-06-12");
    assert!(
        ecrits[0].chemin.ends_with("brm2607_plan_mep_1_2026-05-15.txt"),
        "chemin inattendu : {}",
        ecrits[0].chemin
    );
}
```

Et le helper, dans le même module `tests` :

```rust
fn ligne_plan(cf: &str, mep_id: usize, mep_date: &str) -> crate::plan::LignePlan {
    let d = |iso: &str| chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap();
    crate::plan::LignePlan {
        cf: cf.into(),
        participant: "0225:1".into(),
        jj: 5,
        raison_sociale: "ACME".into(),
        pa: "Serensia".into(),
        mep_id,
        mep_date: d(mep_date),
        run_num: format!("R{mep_id}"),
        run_date: d(mep_date),
        origine: crate::plan::Origine::Auto,
        in_directory: true,
        ..Default::default()
    }
}
```

> Si `LignePlan` n'implémente pas `Default`, remplacer `..Default::default()` par les champs restants — les lire dans `plan.rs` et les recopier. Le helper de `plan_report.rs:421` (`fn ligne(...)`) montre la liste complète pour ce projet.

- [ ] **Step 2: Run test to verify it fails**

```bash
cd client/src-tauri && cargo test un_fichier_de_mep_porte_la_date -- --nocapture
```

Attendu : ÉCHEC à la compilation — `no field 'mep_date' on type 'FichierMep'`.

- [ ] **Step 3: Write minimal implementation**

Dans `commands.rs`, la struct :

```rust
#[derive(Serialize)]
pub struct FichierMep {
    pub chemin: String,
    pub mep_id: usize,
    /// Date de la MEP en ISO, telle qu'elle figure dans le nom du fichier.
    pub mep_date: String,
    pub comptes: usize,
}
```

Puis dans la boucle de `ecrire_fichiers_mep`, au `out.push` :

```rust
        out.push(FichierMep {
            chemin: chemin.display().to_string(),
            mep_id,
            mep_date: mep_date.clone(),
            comptes: comptes.len(),
        });
```

> `mep_date` est déjà une `String` ISO dans la boucle (`let mut meps: Vec<(usize, String)>`, `commands.rs:1151`) : le `clone()` est nécessaire car `format!` l'a empruntée pour le nom juste avant.

- [ ] **Step 4: Run test to verify it passes**

```bash
cd client/src-tauri && cargo test un_fichier_de_mep_porte_la_date
```

Attendu : `test result: ok. 1 passed`.

- [ ] **Step 5: Run the whole suite**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
```

Attendu : aucune régression. ⚠️ Lire la sortie entière, pas seulement la présence de `test result: ok` — une suite rouge peut afficher cette ligne pour d'autres binaires de test.

- [ ] **Step 6: Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): un fichier de MEP remonte sa date"
```

---

## Task 2: `sauver_apres_retouche` remonte les fichiers écrits

Élargissement de type de retour, sans logique nouvelle. **Pas de test dédié** : il n'y a rien à prouver qu'un test pourrait démentir, et les cinq appelants sont couverts exhaustivement par le compilateur. Le comportement observable arrive à la tâche 8.

**Files:**
- Modify: `client/src-tauri/src/commands.rs:1486-1504` (`sauver_apres_retouche`)
- Modify: `client/src-tauri/src/commands.rs:1561`, `:1584`, `:1602`, `:1619` (appelants qui ignorent le nouveau champ)

- [ ] **Step 1: Changer la signature et le retour**

```rust
fn sauver_apres_retouche(
    store: &Arc<Mutex<Store>>,
    input: &Path,
    cfg: &Config,
    lignes: &[crate::plan::LignePlan],
    meta: &crate::store::PlanMeta,
) -> Result<(Vec<FichierMep>, Vec<String>), String> {
    store.lock().unwrap().ecrire_plan(lignes, meta)?;
    let (ecrits, obsoletes) = ecrire_fichiers_mep(input, &cfg.output.dir, lignes)?;
    let entrees = {
        let s = store.lock().unwrap();
        plan_entrees_from_scan(&s, input, cfg, chrono::Utc::now())?
    };
    crate::plan_xlsx::ecrire(
        &chemin_classeur(input, &cfg.output.dir),
        &crate::plan_xlsx::lignes(&entrees, lignes),
    )?;
    Ok((ecrits, obsoletes))
}
```

Compléter le commentaire de doc existant, au-dessus de la fonction, par :

```rust
/// Rend aussi **les fichiers de MEP écrits** : `plan_rapprocher_appliquer` en a
/// besoin pour son rapport, et eux seuls disent combien de comptes chaque
/// fichier porte réellement. Les autres appelants les ignorent.
```

- [ ] **Step 2: Adapter les quatre appelants qui n'en veulent pas**

Aux lignes 1561, 1584, 1602 et 1619 (`plan_ajouter`, `plan_deplacer`, `plan_retirer`, `plan_annuler_retrait`), la dernière expression du bloc devient :

```rust
        sauver_apres_retouche(&store, &input, &cfg, &lignes, &meta).map(|(_, obs)| obs)
```

Idem pour `plan_rapprocher_appliquer` (ligne 1758) — provisoire, la tâche 8 la réécrit.

- [ ] **Step 3: Compiler et lancer la suite**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
```

Attendu : compile, aucune régression.

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): la retouche remonte les fichiers de MEP écrits"
```

---

## Task 3: Le module, son en-tête et son résumé chiffré

**Files:**
- Create: `client/src-tauri/src/rapprochement_report.rs`
- Modify: `client/src-tauri/src/lib.rs` (déclaration du module)

- [ ] **Step 1: Déclarer le module**

Dans `lib.rs`, en gardant l'ordre alphabétique des `pub mod`, juste après `pub mod rapprochement;` :

```rust
pub mod rapprochement_report;
```

- [ ] **Step 2: Write the failing test**

Créer `client/src-tauri/src/rapprochement_report.rs` avec **uniquement** le module de tests ci-dessous, plus les `use` :

```rust
//! Rapport HTML d'un rapprochement appliqué.
//!
//! Module **pur** : aucune I/O, aucune horloge, aucune dépendance à Tauri ni à
//! `commands`. Ce qui vient du disque (noms de fichiers, empreinte) ou de
//! l'horloge (date longue) est fourni tout prêt par l'appelant, comme pour
//! `plan_report`.

use crate::rapprochement::{Action, Ecart, Nature, Rapprochement};
use crate::report::{esc, fmt_int, CSS};

#[cfg(test)]
mod tests {
    use super::*;

    /// Le corps du rapport, feuille de style exclue.
    ///
    /// Le CSS est inliné : chercher une sous-chaîne dans le HTML entier fait
    /// matcher une règle ou un commentaire de style. Le module `plan_report`
    /// s'y est laissé prendre trois fois — assertions vertes sur une fonction
    /// qui ne produisait rien.
    fn corps(html: &str) -> &str {
        html.split("</style>")
            .nth(1)
            .expect("le rapport doit contenir une feuille de style")
    }

    fn vide() -> Rapprochement {
        Rapprochement::default()
    }

    fn donnees<'a>(r: &'a Rapprochement) -> RapprochementReportData<'a> {
        RapprochementReportData {
            fichier_avant: "brm2606.csv",
            fichier_apres: "brm2607.csv",
            empreinte: "9f3c1ab27de4508bb6a1e0f47c25d9836ea15b0c7d42f98e3a6b5c0197de24af",
            date_longue: "mardi 28 juillet 2026",
            version: "1.6.0",
            rapprochement: r,
            fichiers: &[],
            obsoletes: &[],
            origines: origines_vides(),
            annuaire_incomplet: None,
        }
    }

    /// Table d'origines vide, partagée : `donnees()` rend une struct
    /// empruntante, elle ne peut pas emprunter un temporaire local.
    fn origines_vides() -> &'static std::collections::BTreeMap<String, PositionAvant> {
        static VIDE: std::sync::OnceLock<std::collections::BTreeMap<String, PositionAvant>> =
            std::sync::OnceLock::new();
        VIDE.get_or_init(Default::default)
    }

    #[test]
    fn l_entete_nomme_les_deux_fichiers_et_l_empreinte() {
        // Le destinataire compare l'empreinte : sans elle, il ne peut pas
        // vérifier qu'il parle du même fichier que l'émetteur.
        let r = vide();
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(c.contains("brm2606.csv"), "fichier d'origine absent");
        assert!(c.contains("brm2607.csv"), "fichier rapproché absent");
        assert!(c.contains("mardi 28 juillet 2026"), "date absente");
        assert!(
            c.contains("9f3c1ab27de4508bb6a1e0f47c25d9836ea15b0c7d42f98e3a6b5c0197de24af"),
            "empreinte absente"
        );
    }

    #[test]
    fn le_resume_compte_les_retraits_par_motif() {
        // Deux natures différentes mènent au même retrait : le résumé doit
        // les distinguer, sinon « 5 retirés » ne dit pas s'il faut aller
        // regarder l'annuaire ou le fichier.
        let mut r = vide();
        r.inchangees = 143;
        r.ecarts = vec![
            ecart_eligibilite("4100000001"),
            ecart_eligibilite("4100000002"),
            ecart_disparu("4100000003"),
        ];
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(c.contains("comptes retirés"), "libellé des retraits absent");
        assert!(c.contains(">3<"), "total des retraits absent");
        assert!(c.contains("2</b> non éligibles"), "détail non éligibles absent");
        assert!(c.contains("1</b> disparu"), "détail disparus absent");
        assert!(c.contains("143"), "lignes inchangées absentes");
    }

    fn ecart_eligibilite(cf: &str) -> Ecart {
        Ecart {
            cf: cf.into(),
            nature: Nature::EligibilitePerdue {
                avant: "CTC prêt".into(),
                apres: "CTC non prêt".into(),
            },
            action: Action::Retirer {
                motif: "2026-07-28 — CTC non prêt".into(),
            },
            gelee: false,
        }
    }

    fn ecart_disparu(cf: &str) -> Ecart {
        Ecart {
            cf: cf.into(),
            nature: Nature::DisparuDuFichier,
            action: Action::Retirer {
                motif: "absent du fichier rapproché".into(),
            },
            gelee: false,
        }
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : ÉCHEC à la compilation — `cannot find function 'render'`, `cannot find struct 'RapprochementReportData'`.

- [ ] **Step 4: Write minimal implementation**

Au-dessus du `mod tests`, dans le même fichier :

```rust
/// Un fichier de livraison, tel que le rapport en parle : par son nom.
pub struct FichierLivre<'a> {
    pub nom: &'a str,
    pub mep_id: usize,
    pub mep_date: &'a str,
    pub comptes: usize,
}

/// Où se trouvait une ligne avant le rapprochement.
///
/// `Action::Deplacer` ne porte que la **destination**, et `appliquer` mute les
/// lignes en place : après application, le run d'origine n'existe plus nulle
/// part. La commande capture donc ces positions AVANT d'appliquer.
pub struct PositionAvant {
    pub run_num: String,
    /// ISO, comme partout ailleurs dans le projet.
    pub run_date: String,
    pub mep_id: usize,
}

pub struct RapprochementReportData<'a> {
    /// Nom du fichier qui a produit le plan, capturé AVANT réalignement.
    pub fichier_avant: &'a str,
    pub fichier_apres: &'a str,
    /// SHA-256 du fichier rapproché : le destinataire le compare.
    pub empreinte: &'a str,
    /// Déjà formatée par `report::date_fr_longue` — ce module n'a pas d'horloge.
    pub date_longue: &'a str,
    pub version: &'a str,
    pub rapprochement: &'a Rapprochement,
    pub fichiers: &'a [FichierLivre<'a>],
    /// Fichiers de MEP supprimés parce que leur MEP s'est vidée. Noms nus.
    pub obsoletes: &'a [String],
    /// Position d'origine des lignes, capturée AVANT `appliquer`. Clé : n° de CF.
    pub origines: &'a std::collections::BTreeMap<String, PositionAvant>,
    /// Avertissement d'annuaire PPF incomplet, s'il y a lieu.
    pub annuaire_incomplet: Option<&'a str>,
}

/// Écarts portant l'action demandée.
fn par_action<'a>(r: &'a Rapprochement, f: impl Fn(&Action) -> bool) -> Vec<&'a Ecart> {
    r.ecarts.iter().filter(|e| f(&e.action)).collect()
}

/// Écarts retirés dont la nature est celle-ci.
fn retraits_de_nature<'a>(r: &'a Rapprochement, disparus: bool) -> Vec<&'a Ecart> {
    r.ecarts
        .iter()
        .filter(|e| matches!(e.action, Action::Retirer { .. }))
        .filter(|e| matches!(e.nature, Nature::DisparuDuFichier) == disparus)
        .collect()
}

pub fn render(d: &RapprochementReportData) -> String {
    let r = d.rapprochement;
    let retires = par_action(r, |a| matches!(a, Action::Retirer { .. }));
    let inelig = retraits_de_nature(r, false).len();
    let disparus = retraits_de_nature(r, true).len();
    let deplaces = par_action(r, |a| matches!(a, Action::Deplacer { .. })).len();
    let rafraichis = par_action(r, |a| matches!(a, Action::Rafraichir)).len();
    let actives = r.inchangees + r.ecarts.len();

    let avant = esc(d.fichier_avant);
    let apres = esc(d.fichier_apres);

    let mut html = String::with_capacity(16 * 1024);
    html.push_str("<!doctype html>\n<html lang=\"fr\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "<title>Rapprochement du plan — {apres}</title>\n<style>{CSS}{CSS_RAPPRO}</style>\n\
         </head>\n<body>\n<div class=\"page\">\n"
    ));
    html.push_str(&format!(
        "<header>\n<div class=\"wordmark\">SUPER POPAUL</div>\n\
         <h1>Rapprochement du plan de charge</h1>\n\
         <p class=\"meta\">Plan établi sur <b>{avant}</b> · rapproché de <b>{apres}</b> \
         · appliqué le <b>{}</b></p>\n\
         <p class=\"hash\">Empreinte du fichier rapproché : {}</p>\n</header>\n",
        esc(d.date_longue),
        esc(d.empreinte),
    ));

    html.push_str("<section class=\"kpis\">\n");
    html.push_str(&format!(
        "<div class=\"kpi red\"><div class=\"v\">{}</div>\
         <div class=\"l\">comptes retirés</div>\
         <div class=\"abs\"><b>{}</b> non éligibles · <b>{}</b> disparu{}</div></div>\n",
        fmt_int(retires.len() as u64),
        fmt_int(inelig as u64),
        fmt_int(disparus as u64),
        if disparus > 1 { "s" } else { "" },
    ));
    html.push_str(&format!(
        "<div class=\"kpi gold\"><div class=\"v\">{}</div>\
         <div class=\"l\">comptes déplacés</div>\
         <div class=\"abs\">jour de cycle changé</div></div>\n",
        fmt_int(deplaces as u64),
    ));
    html.push_str(&format!(
        "<div class=\"kpi amber\"><div class=\"v\">{}</div>\
         <div class=\"l\">plateformes corrigées</div>\
         <div class=\"abs\">la ligne ne bouge pas</div></div>\n",
        fmt_int(rafraichis as u64),
    ));
    html.push_str(&format!(
        "<div class=\"kpi green\"><div class=\"v\">{}</div>\
         <div class=\"l\">lignes inchangées</div>\
         <div class=\"abs\">sur <b>{}</b> actives</div></div>\n",
        fmt_int(r.inchangees as u64),
        fmt_int(actives as u64),
    ));
    html.push_str("</section>\n");

    html.push_str(&format!(
        "<footer>\n<span>Le détail compte par compte figure dans le classeur \
         du périmètre.</span>\n<span>Super Popaul {}</span>\n</footer>\n\
         </div>\n</body>\n</html>\n",
        esc(d.version),
    ));
    html
}

/// Ajouts de style propres à ce rapport. Repris tels quels de la maquette
/// validée du 28/07/2026 ; rien de `report::CSS` n'est modifié.
const CSS_RAPPRO: &str = r#"
  .warn.danger { border-left-color: var(--red); }
  .warn.danger h2 { color: var(--red); }
  .warn.danger li::marker { color: var(--red); }
  .chg .old { color: var(--muted); }
  .chg .arr { color: var(--muted); padding: 0 5px; }
  .same { color: var(--muted); font-size: 12px; padding-left: 6px; }
  .hash { font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 11.5px; color: var(--muted); word-break: break-all; }
  .todo { background: var(--card); border: 1px solid var(--border);
    border-left: 3px solid var(--pa-autres); border-radius: 8px; padding: 14px 18px; }
  .todo h2 { margin: 0 0 8px; font-size: 13px; text-transform: uppercase;
    letter-spacing: .08em; color: var(--fg); }
  .todo h2::after { display: none; }
  tbody tr.gone td { color: var(--muted); }
  tbody tr.gone .why { font-size: 12px; }
"#;
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : `2 passed`.

- [ ] **Step 6: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): en-tête et résumé du rapport de rapprochement"
```

---

## Task 4: Les tableaux d'écarts par nature

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs`

- [ ] **Step 1: Write the failing tests**

Dans `mod tests` :

```rust
#[test]
fn chaque_nature_a_son_tableau_avec_l_avant_et_l_apres() {
    let mut r = vide();
    r.ecarts = vec![
        ecart_eligibilite("4100000001"),
        ecart_disparu("4100000002"),
        ecart_deplace("4100000003", 8, 15, "R13", "2026-09-22", 3),
        ecart_plateforme("4100000004", "Serensia", "Docoon"),
    ];
    let c = render(&donnees(&r)).to_string();
    let c = corps(&c).to_string();
    assert!(c.contains("éligibilité perdue"), "section éligibilité absente");
    assert!(c.contains("disparus du fichier"), "section disparus absente");
    assert!(c.contains("jour de cycle changé"), "section déplacés absente");
    assert!(c.contains("Plateformes corrigées"), "section plateformes absente");
    // L'avant reste lisible : c'est ce que le destinataire a sous les yeux.
    assert!(c.contains("CTC prêt"), "éligibilité d'avant absente");
    assert!(c.contains("CTC non prêt"), "éligibilité d'après absente");
    assert!(c.contains("Serensia"), "plateforme d'avant absente");
    assert!(c.contains("Docoon"), "plateforme d'après absente");
}

#[test]
fn une_nature_sans_ecart_ne_produit_pas_de_tableau_vide() {
    // Un tableau à en-tête seul se lit « rien à signaler ici » alors qu'il
    // veut dire « cette question ne s'est pas posée ».
    let mut r = vide();
    r.ecarts = vec![ecart_eligibilite("4100000001")];
    let html = render(&donnees(&r));
    let c = corps(&html);
    assert!(c.contains("éligibilité perdue"));
    assert!(!c.contains("disparus du fichier"), "tableau des disparus rendu à vide");
    assert!(!c.contains("jour de cycle changé"), "tableau des déplacés rendu à vide");
    assert!(!c.contains("Plateformes corrigées"), "tableau des plateformes rendu à vide");
}

#[test]
fn un_champ_venu_du_csv_sort_echappe() {
    // Un CSV est une entrée non fiable. `esc` oublié sur un seul champ
    // suffit à injecter du balisage dans un document qu'on transmet.
    let mut r = vide();
    r.ecarts = vec![ecart_plateforme("<script>alert(1)</script>", "A&B", "C<D")];
    let html = render(&donnees(&r));
    let c = corps(&html);
    assert!(!c.contains("<script>"), "le n° de CF n'est pas échappé");
    assert!(c.contains("&lt;script&gt;"), "le n° de CF devrait être échappé");
    assert!(c.contains("A&amp;B"), "la plateforme d'avant n'est pas échappée");
    assert!(c.contains("C&lt;D"), "la plateforme d'après n'est pas échappée");
}

#[test]
fn un_deplacement_qui_ne_change_pas_de_run_ne_repete_pas_la_valeur() {
    // Le calcul produit un écart dès que le jour lu diffère, même si les
    // deux jours tombent dans le même run. Écrire « Run 16 → Run 16 » est
    // exact mais illisible : le lecteur cherche une différence qui n'existe
    // pas.
    let mut r = vide();
    r.ecarts = vec![ecart_deplace("4100245920", 30, 28, "R16", "2026-10-13", 5)];
    let mut origines = std::collections::BTreeMap::new();
    origines.insert(
        "4100245920".to_string(),
        PositionAvant {
            run_num: "R16".into(),
            run_date: "2026-10-13".into(),
            mep_id: 5,
        },
    );
    let mut d = donnees(&r);
    d.origines = &origines;
    let html = render(&d);
    let c = corps(&html);
    assert!(c.contains("même run"), "le cas « même run » doit se dire");
    assert_eq!(
        c.matches("Run R16").count(),
        1,
        "le run ne doit apparaître qu'une fois"
    );
    // Le jour, lui, reste en avant → après : c'est là qu'est le changement.
    assert!(c.contains("30"), "le jour d'avant doit rester lisible");
    assert!(c.contains("28"), "le jour d'après doit être là");
}

#[test]
fn un_deplacement_vers_un_autre_run_montre_les_deux() {
    let mut r = vide();
    r.ecarts = vec![ecart_deplace("4100240115", 8, 15, "R13", "2026-09-22", 3)];
    let mut origines = std::collections::BTreeMap::new();
    origines.insert(
        "4100240115".to_string(),
        PositionAvant {
            run_num: "R12".into(),
            run_date: "2026-09-15".into(),
            mep_id: 3,
        },
    );
    let mut d = donnees(&r);
    d.origines = &origines;
    let html = render(&d);
    let c = corps(&html);
    assert!(c.contains("R12"), "le run d'origine doit figurer");
    assert!(c.contains("R13"), "le run d'arrivée doit figurer");
    assert!(!c.contains("même run"), "les runs diffèrent");
}

fn ecart_deplace(
    cf: &str,
    avant: u8,
    apres: u8,
    run: &str,
    run_date: &str,
    mep_id: usize,
) -> Ecart {
    let d = |iso: &str| chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap();
    Ecart {
        cf: cf.into(),
        nature: Nature::JourChange { avant, apres },
        action: Action::Deplacer {
            run_num: run.into(),
            run_date: d(run_date),
            mep_id,
            mep_date: d(run_date),
        },
        gelee: false,
    }
}

fn ecart_plateforme(cf: &str, avant: &str, apres: &str) -> Ecart {
    Ecart {
        cf: cf.into(),
        nature: Nature::PlateformeChangee {
            avant: avant.into(),
            apres: apres.into(),
        },
        action: Action::Rafraichir,
        gelee: false,
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : 5 échecs — les sections n'existent pas encore.

- [ ] **Step 3: Write the implementation**

Ajouter les helpers de rendu, au-dessus de `render` :

```rust
/// Une date ISO rendue en jour/mois/année. Le rapport est lu par des humains.
fn date_fr(iso: &str) -> String {
    match (iso.get(0..4), iso.get(5..7), iso.get(8..10)) {
        (Some(a), Some(m), Some(j)) => format!("{j}/{m}/{a}"),
        _ => iso.to_string(),
    }
}

/// Un jour de cycle, ou son absence. `0` est la sentinelle de
/// `rapprochement::calculer` pour un jour que le fichier ne permet pas de
/// lire : hors du domaine 1–31, il ne s'affiche jamais tel quel.
fn jour(j: u8) -> String {
    if j == 0 {
        "illisible".into()
    } else {
        j.to_string()
    }
}

/// Cellule « avant → après ». L'ancienne valeur reste lisible.
fn chg(avant: &str, apres: &str) -> String {
    format!(
        "<td class=\"chg\"><span class=\"old\">{}</span>\
         <span class=\"arr\">→</span>{}</td>",
        esc(avant),
        esc(apres)
    )
}

/// Le run d'arrivée d'un déplacement : son numéro nu (pour comparer), son
/// libellé lisible, et sa MEP.
///
/// **Rien n'est échappé ici** : `chg` échappe ses deux arguments, et échapper
/// en amont produirait `&amp;amp;`. L'échappement se fait au point d'insertion.
fn destination(a: &Action) -> Option<(String, String, usize)> {
    match a {
        Action::Deplacer { run_num, run_date, mep_id, .. } => Some((
            run_num.clone(),
            format!("Run {} — {}", run_num, date_fr(&run_date.to_string())),
            *mep_id,
        )),
        _ => None,
    }
}

/// Ouvre une section titrée. Rien n'est écrit si la liste est vide : un
/// tableau à en-tête seul dit « rien à signaler », pas « sans objet ».
fn section(html: &mut String, titre: &str, sous_titre: &str, entetes: &[&str], vide: bool) {
    if vide {
        return;
    }
    html.push_str(&format!("<h2>{}</h2>\n", esc(titre)));
    html.push_str(&format!("<p class=\"h2sub\">{}</p>\n", esc(sous_titre)));
    html.push_str("<div class=\"tbl\">\n<table>\n<thead><tr>");
    for e in entetes {
        html.push_str(&format!("<th>{}</th>", esc(e)));
    }
    html.push_str("</tr></thead>\n<tbody>\n");
}

fn fin_section(html: &mut String, vide: bool) {
    if !vide {
        html.push_str("</tbody>\n</table>\n</div>\n");
    }
}
```

Puis, dans `render`, entre la section `kpis` et le `footer` :

```rust
    // ① Éligibilité perdue.
    let inelig_l = retraits_de_nature(r, false);
    section(
        &mut html,
        "Comptes retirés — éligibilité perdue",
        "Le compte est au plan mais le fichier ne le déclare plus éligible.",
        &["N° de CF", "Éligibilité", "Motif"],
        inelig_l.is_empty(),
    );
    for e in &inelig_l {
        let (avant, apres) = match &e.nature {
            Nature::EligibilitePerdue { avant, apres } => (avant.as_str(), apres.as_str()),
            _ => ("", ""),
        };
        let motif = match &e.action {
            Action::Retirer { motif } => motif.as_str(),
            _ => "",
        };
        html.push_str(&format!(
            "<tr><td>{}</td>{}<td>{}</td></tr>\n",
            esc(&e.cf),
            chg(avant, apres),
            esc(motif),
        ));
    }
    fin_section(&mut html, inelig_l.is_empty());

    // ② Disparus du fichier.
    let disparus_l = retraits_de_nature(r, true);
    section(
        &mut html,
        "Comptes retirés — disparus du fichier",
        "Le compte était au plan et n'apparaît plus dans le fichier rapproché.",
        &["N° de CF", "Motif"],
        disparus_l.is_empty(),
    );
    for e in &disparus_l {
        let motif = match &e.action {
            Action::Retirer { motif } => motif.as_str(),
            _ => "",
        };
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td></tr>\n",
            esc(&e.cf),
            esc(motif)
        ));
    }
    fin_section(&mut html, disparus_l.is_empty());

    // ③ Déplacés.
    let deplaces_l = par_action(r, |a| matches!(a, Action::Deplacer { .. }));
    section(
        &mut html,
        "Comptes déplacés — jour de cycle changé",
        "Le fichier annonce un autre jour de facturation : la ligne suit son cycle.",
        &["N° de CF", "Jour", "Run", "MEP"],
        deplaces_l.is_empty(),
    );
    for e in &deplaces_l {
        let (javant, japres) = match &e.nature {
            Nature::JourChange { avant, apres } => (*avant, *apres),
            _ => (0, 0),
        };
        // Un `Deplacer` a toujours une destination ; la branche par défaut
        // n'existe que pour ne pas paniquer si un variant s'y glisse un jour.
        let (run_num, run_lbl, mep_id) = destination(&e.action)
            .unwrap_or_else(|| (String::new(), "—".into(), 0));
        let avant = d.origines.get(&e.cf);
        let cell_run = match avant {
            // Le jour a changé sans changer de run : ne pas écrire deux fois
            // la même valeur, dire que le déplacement a été évalué.
            Some(p) if p.run_num == run_num => format!(
                "<td>{} <span class=\"same\">même run</span></td>",
                esc(&run_lbl)
            ),
            Some(p) => chg(
                &format!("Run {} — {}", p.run_num, date_fr(&p.run_date)),
                &run_lbl,
            ),
            None => format!("<td>{}</td>", esc(&run_lbl)),
        };
        let cell_mep = match avant {
            Some(p) if p.mep_id != mep_id => chg(&p.mep_id.to_string(), &mep_id.to_string()),
            _ => format!("<td>{mep_id}</td>"),
        };
        html.push_str(&format!(
            "<tr><td>{}</td>{}{}{}</tr>\n",
            esc(&e.cf),
            chg(&jour(javant), &jour(japres)),
            cell_run,
            cell_mep,
        ));
    }
    fin_section(&mut html, deplaces_l.is_empty());

    // ④ Plateformes corrigées.
    let plat_l = par_action(r, |a| matches!(a, Action::Rafraichir));
    section(
        &mut html,
        "Plateformes corrigées",
        "Le champ est mis à jour, la ligne ne change ni de run ni de MEP.",
        &["N° de CF", "Plateforme"],
        plat_l.is_empty(),
    );
    for e in &plat_l {
        let (avant, apres) = match &e.nature {
            Nature::PlateformeChangee { avant, apres } => (avant.as_str(), apres.as_str()),
            _ => ("", ""),
        };
        html.push_str(&format!(
            "<tr><td>{}</td>{}</tr>\n",
            esc(&e.cf),
            chg(avant, apres)
        ));
    }
    fin_section(&mut html, plat_l.is_empty());
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : `7 passed`.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs
git commit -m "feat(superpopaul): les écarts du rapprochement par nature"
```

---

## Task 5: Retraits sur MEP transmise, et « à traiter à la main »

Les deux sections que la maquette met en avant, et les deux règles de rendu qui les accompagnent.

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn un_retrait_sur_mep_transmise_a_sa_propre_section() {
    // Les fichiers sont cumulatifs : le destinataire a une version
    // antérieure de ce fichier entre les mains. Noyer ce cas dans le
    // tableau général est le principal moyen de le rater.
    let mut r = vide();
    let mut e = ecart_eligibilite("4100238877");
    e.gelee = true;
    r.ecarts = vec![e, ecart_eligibilite("4100241902")];
    let html = render(&donnees(&r));
    let c = corps(&html);
    assert!(
        c.contains("mise en production déjà transmise"),
        "section des retraits sur MEP livrée absente"
    );
    let avant_sections = c.split("<h2>").next().unwrap_or("");
    assert!(
        avant_sections.contains("4100238877"),
        "le compte gelé doit apparaître AVANT les tableaux d'écarts"
    );
}

#[test]
fn sans_retrait_gele_la_section_d_alerte_n_existe_pas() {
    let mut r = vide();
    r.ecarts = vec![ecart_eligibilite("4100241902")];
    let html = render(&donnees(&r));
    assert!(!corps(&html).contains("mise en production déjà transmise"));
}

#[test]
fn un_jour_illisible_se_dit_en_toutes_lettres() {
    // `apres: 0` est une sentinelle hors du domaine 1–31. L'afficher comme
    // un chiffre ferait lire « le compte passe au jour 0 ».
    let mut r = vide();
    r.ecarts = vec![Ecart {
        cf: "4100252009".into(),
        nature: Nature::JourChange { avant: 9, apres: 0 },
        action: Action::Signaler,
        gelee: false,
    }];
    let html = render(&donnees(&r));
    let c = corps(&html);
    assert!(c.contains("illisible"), "le jour illisible doit se dire");
    assert!(!c.contains("jour 0"), "la sentinelle ne doit jamais s'afficher");
    assert!(!c.contains(">0<"), "la sentinelle ne doit jamais s'afficher");
}

#[test]
fn un_signalement_n_est_pas_compte_parmi_les_changements() {
    // « Signaler » ne mute rien. Le compter dans les déplacés ferait
    // annoncer un mouvement qui n'a pas eu lieu.
    let mut r = vide();
    r.ecarts = vec![Ecart {
        cf: "4100251774".into(),
        nature: Nature::JourChange { avant: 9, apres: 24 },
        action: Action::Signaler,
        gelee: true,
    }];
    let html = render(&donnees(&r));
    let c = corps(&html);
    assert!(c.contains("À traiter à la main"), "section des signalements absente");
    assert!(
        !c.contains("jour de cycle changé"),
        "un signalement ne doit pas produire de tableau de déplacés"
    );
    assert!(
        c.contains("<div class=\"v\">0</div>"),
        "les compteurs de changements doivent rester à zéro"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : 4 échecs.

- [ ] **Step 3: Write the implementation**

`jour()` a été écrit en tâche 4, avec les autres helpers de rendu. Il n'y a rien à ajouter ici de ce côté : ce qui manque, ce sont les deux sections.

Dans `render`, **juste après la section `kpis`** et donc avant tous les `<h2>` :

```rust
    // Retraits portant sur une MEP déjà transmise. Avant tout le reste : les
    // fichiers étant cumulatifs, le destinataire a une version antérieure.
    let geles: Vec<&Ecart> = retires
        .iter()
        .copied()
        .filter(|e| e.gelee)
        .collect();
    if !geles.is_empty() {
        html.push_str(
            "<section class=\"warn danger\">\n\
             <h2>Retrait portant sur une mise en production déjà transmise</h2>\n<ul>\n",
        );
        for e in &geles {
            let motif = match &e.action {
                Action::Retirer { motif } => motif.as_str(),
                _ => "",
            };
            html.push_str(&format!(
                "<li>Le compte <b>{}</b> figurait dans un fichier qui vous a déjà été \
                 transmis. Les fichiers étant cumulatifs, il ne figure plus dans aucun \
                 fichier de ce lot. Motif : <b>{}</b>.</li>\n",
                esc(&e.cf),
                esc(motif),
            ));
        }
        html.push_str("</ul>\n</section>\n");
    }
```

Et **après** les quatre tableaux d'écarts :

```rust
    // Ce que le rapprochement n'a pas tranché.
    let signales = par_action(r, |a| matches!(a, Action::Signaler));
    if !signales.is_empty() {
        html.push_str("<section class=\"todo\">\n<h2>À traiter à la main</h2>\n<ul>\n");
        for e in &signales {
            let quoi = match &e.nature {
                Nature::JourChange { avant, apres } if *apres == 0 => format!(
                    "annonce un jour de cycle <b>illisible</b> dans le fichier rapproché \
                     (il était à <b>{}</b>). La ligne n'a pas été déplacée.",
                    jour(*avant)
                ),
                Nature::JourChange { avant, apres } => format!(
                    "voit son jour de cycle passer de <b>{}</b> à <b>{}</b>{}. \
                     La ligne n'a pas été déplacée.",
                    jour(*avant),
                    jour(*apres),
                    if e.gelee {
                        " alors qu'il est gelé (mise en production déjà transmise)"
                    } else {
                        ", sans run disponible pour l'accueillir"
                    },
                ),
                Nature::EligibilitePerdue { avant, apres } => {
                    format!("passe de <b>{}</b> à <b>{}</b>.", esc(avant), esc(apres))
                }
                Nature::PlateformeChangee { avant, apres } => {
                    format!("change de plateforme : <b>{}</b> → <b>{}</b>.", esc(avant), esc(apres))
                }
                Nature::DisparuDuFichier => {
                    "n'apparaît plus dans le fichier rapproché.".to_string()
                }
            };
            html.push_str(&format!(
                "<li>Le compte <b>{}</b> {}</li>\n",
                esc(&e.cf),
                quoi
            ));
        }
        html.push_str("</ul>\n</section>\n");
    }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : `11 passed`.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs
git commit -m "feat(superpopaul): alerte MEP transmise et signalements du rapport"
```

---

## Task 6: Les deux blocs d'avertissements

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn les_avertissements_du_calcul_et_de_l_annuaire_ne_se_confondent_pas() {
    // Décision verrouillée au chantier précédent : les premiers décrivent ce
    // que le rapprochement FAIT, le second prévient qu'il est INCOMPLET.
    // Les fondre ferait disparaître la nuance.
    let mut r = vide();
    r.avertissements = vec!["le run 12 n'accueille plus de compte".into()];
    let mut d = donnees(&r);
    d.annuaire_incomplet = Some("l'annuaire PPF a été construit par cumul de 3 fichiers");
    let html = render(&d);
    let c = corps(&html);
    assert!(c.contains("le run 12 n'accueille plus de compte"));
    assert!(c.contains("l'annuaire PPF a été construit par cumul de 3 fichiers"));
    assert!(c.contains("Annuaire PPF incomplet"), "titre propre à l'annuaire absent");
    assert_eq!(
        c.matches("class=\"warn\"").count(),
        2,
        "les deux avertissements doivent vivre dans deux encadrés distincts"
    );
}

#[test]
fn sans_avertissement_aucun_encadre_n_est_rendu() {
    let r = vide();
    let html = render(&donnees(&r));
    assert!(!corps(&html).contains("class=\"warn\""));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : 2 échecs.

- [ ] **Step 3: Write the implementation**

Dans `render`, **entre la section `kpis` et le bloc des retraits gelés** :

```rust
    if !r.avertissements.is_empty() {
        html.push_str("<section class=\"warn\">\n<h2>Avertissements</h2>\n<ul>\n");
        for a in &r.avertissements {
            html.push_str(&format!("<li>{}</li>\n", esc(a)));
        }
        html.push_str("</ul>\n</section>\n");
    }
    if let Some(a) = d.annuaire_incomplet {
        html.push_str(&format!(
            "<section class=\"warn\">\n<h2>Annuaire PPF incomplet</h2>\n<ul>\n\
             <li>{}<br><b>Les éligibilités PPF perdues ne sont pas visibles</b> — \
             le compte « 0 éligibilité perdue » ne vaut que pour le verdict CTC.</li>\n\
             </ul>\n</section>\n",
            esc(a)
        ));
    }
```

> L'encadré `warn danger` de la tâche 5 porte deux classes : `matches("class=\"warn\"")`
> le compterait aussi. Le test ci-dessus ne met aucun écart gelé, il n'est donc
> pas exposé — mais si un futur test combine les deux, compter `class="warn"\n`
> ou distinguer sur `warn danger` sera nécessaire.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : `13 passed`.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs
git commit -m "feat(superpopaul): avertissements du rapport de rapprochement"
```

---

## Task 7: Les fichiers de livraison produits

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn les_fichiers_produits_sont_listes_avec_leurs_comptes() {
    let r = vide();
    let fichiers = vec![
        FichierLivre { nom: "brm2607_plan_mep_1_2026-05-15.txt", mep_id: 1,
                       mep_date: "2026-05-15", comptes: 28 },
        FichierLivre { nom: "brm2607_plan_mep_2_2026-06-12.txt", mep_id: 2,
                       mep_date: "2026-06-12", comptes: 61 },
    ];
    let mut d = donnees(&r);
    d.fichiers = &fichiers;
    let html = render(&d);
    let c = corps(&html);
    assert!(c.contains("brm2607_plan_mep_1_2026-05-15.txt"));
    assert!(c.contains("15/05/2026"), "la date de MEP doit être lisible");
    assert!(c.contains(">28<"), "le nombre de comptes doit figurer");
    assert!(c.contains(">61<"));
}

#[test]
fn un_fichier_supprime_est_dit_supprime() {
    // Une MEP vidée par les retraits perd son fichier. La ligne existe pour
    // dire qu'elle n'existe plus : le destinataire doit jeter sa copie.
    let r = vide();
    let obsoletes = vec!["brm2607_plan_mep_6_2026-11-27.txt".to_string()];
    let mut d = donnees(&r);
    d.obsoletes = &obsoletes;
    let html = render(&d);
    let c = corps(&html);
    assert!(c.contains("brm2607_plan_mep_6_2026-11-27.txt"));
    assert!(c.contains("supprimé"), "la raison de la disparition doit se lire");
}

#[test]
fn sans_fichier_ni_obsolete_la_section_n_existe_pas() {
    let r = vide();
    let html = render(&donnees(&r));
    assert!(!corps(&html).contains("Fichiers de livraison"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : 3 échecs.

- [ ] **Step 3: Write the implementation**

Dans `render`, **après** la section « à traiter à la main » et avant le `footer` :

```rust
    if !d.fichiers.is_empty() || !d.obsoletes.is_empty() {
        html.push_str("<h2>Fichiers de livraison produits</h2>\n");
        html.push_str(
            "<p class=\"h2sub\">Les fichiers sont cumulatifs : celui de la MEP <i>n</i> \
             contient les comptes des MEP 1 à <i>n</i>. Ils remplacent ceux du lot \
             précédent.</p>\n",
        );
        html.push_str(
            "<div class=\"tbl\">\n<table>\n<thead><tr><th>Fichier</th><th>MEP</th>\
             <th>Date de MEP</th><th class=\"num\">Comptes</th></tr></thead>\n<tbody>\n",
        );
        for f in d.fichiers {
            html.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td>\
                 <td class=\"num\"><b>{}</b></td></tr>\n",
                esc(f.nom),
                f.mep_id,
                date_fr(f.mep_date),
                fmt_int(f.comptes as u64),
            ));
        }
        for o in d.obsoletes {
            html.push_str(&format!(
                "<tr class=\"gone\"><td>{}</td><td>—</td><td>—</td>\
                 <td class=\"num why\">supprimé — plus aucun compte</td></tr>\n",
                esc(o),
            ));
        }
        html.push_str("</tbody>\n</table>\n</div>\n");
    }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test rapprochement_report
```

Attendu : `16 passed`.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs
git commit -m "feat(superpopaul): les fichiers de livraison dans le rapport"
```

---

## Task 8: Écrire le rapport à l'application

**Files:**
- Modify: `client/src-tauri/src/commands.rs:1730-1762` (`plan_rapprocher_appliquer`)
- Modify: `client/src-tauri/src/commands.rs` (nouvelle struct de retour + helper de chemin)
- Test: `client/src-tauri/src/commands.rs` (module `tests`)

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn le_rapport_de_rapprochement_suit_le_repertoire_de_sortie() {
    let p = chemin_rapport_rapprochement(
        Path::new("/data/brm2607.csv"),
        "sortie",
        "2026-07-28_143205",
    );
    assert!(
        p.ends_with("brm2607_rapprochement_2026-07-28_143205.html"),
        "nom inattendu : {p:?}"
    );
    assert!(p.to_string_lossy().contains("sortie"), "répertoire ignoré : {p:?}");
}

#[test]
fn le_rapport_echappe_au_menage_des_fichiers_de_mep() {
    // `fichiers_obsoletes` supprime tout `<souche>_plan_mep_*.txt` qu'il n'a
    // pas écrit. Un rapport qui tomberait dans ce filtre disparaîtrait au
    // rapprochement suivant — silencieusement, puisqu'il est conservé.
    let presents = vec![
        "brm2607_rapprochement_2026-07-28_143205.html".to_string(),
        "brm2607_plan_mep_1_2026-05-15.txt".to_string(),
    ];
    let ecrits: HashSet<String> = HashSet::new();
    let obsoletes = fichiers_obsoletes(&presents, &ecrits, "brm2607");
    assert_eq!(
        obsoletes,
        vec!["brm2607_plan_mep_1_2026-05-15.txt"],
        "le rapport ne doit jamais être sélectionné pour suppression"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd client/src-tauri && cargo test le_rapport_
```

Attendu : le premier échoue à la compilation (`chemin_rapport_rapprochement` inconnue), le second passe déjà — c'est un test de non-régression, il **doit** être vert dès maintenant et le rester.

- [ ] **Step 3: Write the implementation**

À côté de `chemin_classeur` (`commands.rs:684`) :

```rust
/// Chemin du rapport d'un rapprochement. Horodaté **à la seconde** : le
/// document est transmis et conservé, deux rapprochements rapprochés dans le
/// temps ne doivent pas s'écraser. Le préfixe évite `_plan_mep_`, que
/// `fichiers_obsoletes` sélectionne pour suppression.
fn chemin_rapport_rapprochement(input: &Path, dir: &str, horodatage: &str) -> PathBuf {
    resolved_out_dir(input, dir).join(format!(
        "{}_rapprochement_{horodatage}.html",
        input.file_stem().unwrap_or_default().to_string_lossy()
    ))
}
```

La struct de retour, à côté des autres `#[derive(Serialize)]` :

```rust
#[derive(Serialize)]
pub struct RapprochementApplique {
    /// Fichiers de MEP supprimés parce que leur MEP s'est vidée.
    pub obsoletes: Vec<String>,
    /// Chemin du rapport écrit. Le lot ne part jamais sans sa note.
    pub rapport: String,
}
```

Puis `plan_rapprocher_appliquer` :

```rust
#[tauri::command]
pub async fn plan_rapprocher_appliquer(
    state: State<'_, AppState>,
    empreinte: String,
) -> Result<RapprochementApplique, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        // `annuaire_incomplet` n'est plus ignoré : sans lui, le rapport
        // affirmerait une exhaustivité qu'il n'a pas — « 0 éligibilité perdue »
        // peut vouloir dire « l'annuaire ne sait pas la voir ».
        let (r, courante, mut lignes, mut meta, annuaire_incomplet) =
            calculer_rapprochement(&store, &input, &cfg)?;
        if courante != empreinte {
            return Err("le fichier a changé depuis le calcul — relance le rapprochement \
                        avant d'appliquer"
                .into());
        }
        // Capturé AVANT le réalignement ci-dessous, qui l'écrase : le rapport
        // doit nommer le fichier d'origine.
        let fichier_avant = meta.fichier.clone();
        // Idem pour les positions : `appliquer` mute les lignes en place, et
        // l'écart ne porte que la destination. Sans cette photo, le rapport ne
        // peut pas dire d'où vient un compte déplacé.
        let origines: std::collections::BTreeMap<String, crate::rapprochement_report::PositionAvant> =
            lignes
                .iter()
                .map(|l| {
                    (
                        l.cf.clone(),
                        crate::rapprochement_report::PositionAvant {
                            run_num: l.run_num.clone(),
                            run_date: l.run_date.to_string(),
                            mep_id: l.mep_id,
                        },
                    )
                })
                .collect();
        let maintenant = chrono::Utc::now().timestamp();
        crate::rapprochement::appliquer(&mut lignes, &r, maintenant)?;
        meta.fichier = input
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        meta.hash = courante.clone();
        meta.rapproche_le = Some(maintenant);
        let (ecrits, obsoletes) = sauver_apres_retouche(&store, &input, &cfg, &lignes, &meta)?;


        // Le rapport décrit les fichiers RÉELLEMENT écrits ci-dessus : les
        // produire d'abord évite deux calculs du même nombre de comptes.
        let noms_obsoletes: Vec<String> = obsoletes
            .iter()
            .map(|c| nom_de_fichier(c))
            .collect();
        let fichiers: Vec<crate::rapprochement_report::FichierLivre> = ecrits
            .iter()
            .map(|f| crate::rapprochement_report::FichierLivre {
                nom: nom_de_fichier_str(&f.chemin),
                mep_id: f.mep_id,
                mep_date: &f.mep_date,
                comptes: f.comptes,
            })
            .collect();
        let local = chrono::Local::now();
        let html = crate::rapprochement_report::render(
            &crate::rapprochement_report::RapprochementReportData {
                fichier_avant: &fichier_avant,
                fichier_apres: &meta.fichier,
                empreinte: &meta.hash,
                date_longue: &report::date_fr_longue(&local),
                version: env!("CARGO_PKG_VERSION"),
                rapprochement: &r,
                fichiers: &fichiers,
                obsoletes: &noms_obsoletes,
                origines: &origines,
                annuaire_incomplet: annuaire_incomplet.as_deref(),
            },
        );
        let out = chemin_rapport_rapprochement(
            &input,
            &cfg.output.dir,
            &local.format("%Y-%m-%d_%H%M%S").to_string(),
        );
        // Le plan et les fichiers sont déjà écrits : l'erreur doit dire les
        // DEUX choses, sinon l'utilisateur croit à un échec total et relance —
        // or relancer recalculerait un rapprochement sans écart, et le
        // document serait perdu.
        std::fs::write(&out, html).map_err(|e| {
            format!("Le rapprochement a été appliqué, mais le rapport n'a pas pu être écrit : {e}")
        })?;
        Ok(RapprochementApplique {
            obsoletes,
            rapport: out.display().to_string(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
```

Et le helper de nom, à côté de `chemin_rapport_rapprochement` :

```rust
/// Le nom d'un fichier depuis son chemin. Le rapport nomme les fichiers, il
/// ne parle jamais de chemins absolus — ceux de la machine qui a produit le
/// lot n'ont aucun sens pour qui le reçoit.
fn nom_de_fichier_str(chemin: &str) -> &str {
    chemin.rsplit(['/', '\\']).next().unwrap_or(chemin)
}

fn nom_de_fichier(chemin: &str) -> String {
    nom_de_fichier_str(chemin).to_string()
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
```

Attendu : tout vert. Si `report::date_fr_longue` n'est pas visible depuis `commands.rs`, l'appeler en `crate::report::date_fr_longue` — `plan_rapport` (`commands.rs:1794`) montre la forme en usage.

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): le rapprochement écrit son rapport"
```

---

## Task 9: Le bandeau et les doublures JS

⚠️ **Changement de forme d'un retour de commande.** Les doublures JS renvoient `[]` : le compilateur Rust ne les voit pas, et une doublure qui ment fait porter au code des états impossibles.

**Files:**
- Modify: `client/src/app.js:2453-2467` (`compteRenduRapprochement`)
- Modify: `client/src/app.js:2502` (l'appel)
- Modify: `client/tests/rapprochement.test.js:57`, `:69`, `:93`

- [ ] **Step 1: Vérifier qu'aucune autre doublure ne parle de cette commande**

```bash
cd client && grep -rn "plan_rapprocher_appliquer" tests/ src/
```

Attendu : les trois emplacements listés ci-dessus, plus l'appel dans `app.js`. Si un autre apparaît, le traiter aussi.

- [ ] **Step 2: Write the failing test**

Dans `client/tests/rapprochement.test.js`, changer la doublure :

```js
    if (cmd === "plan_rapprocher_appliquer")
      return ctx.evaluer('({ obsoletes: [], rapport: "/sortie/brm2607_rapprochement_2026-07-28_143205.html" })');
```

> `ctx.evaluer` construit la valeur **dans le realm du faux DOM** : une valeur
> littérale du realm de test échouerait les comparaisons d'identité. Voir
> `client/tests/dom_shim.js`.

Puis ajouter un test, en réutilisant les helpers déjà en tête du fichier
(`ecran()`, `boutonRapprocher`, `boutonModale`, `ctx.repondreAux`,
`ctx.evaluer`) :

```js
test("le compte rendu nomme le rapport écrit", async () => {
  const ctx = ecran();
  const CHEMIN = "/data/sortie/brm2607_rapprochement_2026-07-28_143205.html";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [ECART], inchangees: 0, avertissements: [] },
        empreinte: "peu importe ici",
        annuaire_incomplet: null,
      })})`);
    if (cmd === "plan_lignes") return ctx.evaluer(`(${JSON.stringify([ligne("CF1")])})`);
    if (cmd === "plan_rapprocher_appliquer")
      return ctx.evaluer(`(${JSON.stringify({ obsoletes: [], rapport: CHEMIN })})`);
    return null;
  });

  await boutonRapprocher(ctx.$).click();
  await boutonModale(ctx.$, "Appliquer").click();

  const texte = String(ctx.$("plan-banner").children?.[0] ?? "");
  assert.match(texte, /brm2607_rapprochement_2026-07-28_143205\.html/,
    "le bandeau doit nommer le rapport : une fois la modale fermée, c'est la seule trace du livrable");
  assert.ok(!texte.includes("/data/sortie/"),
    "le chemin de la machine qui a produit le lot n'apprend rien : seul le nom compte");
});
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cd client && node --test "tests/*.test.js"
```

Attendu : le nouveau test échoue (le bandeau ne nomme rien), et les assertions de `:69`/`:93` peuvent casser si elles inspectaient la valeur rendue.

- [ ] **Step 4: Write the implementation**

Dans `app.js`, l'appel :

```js
        const { obsoletes, rapport } = await invoke("plan_rapprocher_appliquer", { empreinte });
        compteRenduRapprochement(rapprochement, obsoletes, rapport);
```

Et la fonction, dont le commentaire de tête est à reprendre :

```js
/** Le compte rendu s'appuie sur le rapprochement DÉJÀ affiché et sur les
 *  obsolètes + le chemin du rapport, seul retour de
 *  `plan_rapprocher_appliquer` — le nombre de fichiers de MEP réécrits n'est
 *  pas remonté par le backend, et ne se devine pas : on ne l'affiche donc pas.
 *  Le rapport, lui, se nomme : c'est la seule trace du livrable une fois le
 *  bandeau parti. */
function compteRenduRapprochement(rapprochement, obsoletes, rapport) {
  const g = grouperEcarts(rapprochement.ecarts);
  const parts = [];
  const retraits = g.eligibilite.length + g.disparus.length;
  if (retraits) parts.push(`${fmtN(retraits)} compte(s) retiré(s)`);
  if (g.deplaces.length) parts.push(`${fmtN(g.deplaces.length)} déplacé(s)`);
  if (g.plateforme.length) parts.push(`${fmtN(g.plateforme.length)} plateforme(s) corrigée(s)`);
  let texte = parts.length ? `✓ Rapprochement appliqué : ${parts.join(", ")}.` : "✓ Rapprochement appliqué.";
  const noms = (obsoletes ?? []).map((c) => c.split(/[/\\]/).pop());
  if (noms.length) texte += ` ${noms.length} fichier(s) obsolète(s) supprimé(s) : ${noms.join(", ")}.`;
  if (rapport) texte += ` Rapport : ${rapport.split(/[/\\]/).pop()}.`;
  planBanner("ok", texte);
}
```

> `planBanner` reçoit du texte, pas du HTML — vérifier qu'il pose bien un
> `textContent`. Un nom de fichier vient indirectement du CSV : jamais
> d'`innerHTML` avec.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd client && node --test "tests/*.test.js"
```

Attendu : tout vert.

- [ ] **Step 6: Commit**

```bash
git add client/src/app.js client/tests/rapprochement.test.js
git commit -m "feat(superpopaul): le bandeau nomme le rapport de rapprochement"
```

---

## Task 10: Passe de mutation sur le nouveau module

Sur ce projet, chaque tâche a livré au moins un test incapable d'échouer. Le but n'est pas de trouver des tests manquants mais des tests qui ne prouvent rien.

**Files:**
- Modify: `client/src-tauri/src/rapprochement_report.rs` (tests durcis)

- [ ] **Step 1: Appliquer les mutations une à une**

Pour chacune, modifier le code, lancer `cargo test rapprochement_report`, **noter si la suite reste verte**, puis annuler la modification.

| # | Mutation | Doit faire rougir |
|---|---|---|
| 1 | `jour()` rend toujours `j.to_string()` | le test du jour illisible |
| 2 | `jour()` rend toujours `"illisible"` | les tests des déplacés |
| 3 | La section « MEP déjà transmise » ne filtre plus sur `e.gelee` | `sans_retrait_gele_la_section_d_alerte_n_existe_pas` |
| 4 | La section « MEP déjà transmise » est rendue après les tableaux | `un_retrait_sur_mep_transmise_a_sa_propre_section` |
| 5 | `retraits_de_nature` inverse son booléen | le résumé chiffré |
| 6 | `par_action(Signaler)` devient `par_action(Deplacer)` | les signalements |
| 7 | `esc()` retiré sur le n° de CF | l'échappement |
| 8 | `esc()` retiré sur la plateforme d'avant | l'échappement |
| 9 | `date_fr` rend l'ISO tel quel | les fichiers de livraison |
| 10 | `section()` rend l'en-tête même quand `vide` est vrai | `une_nature_sans_ecart_ne_produit_pas_de_tableau_vide` |
| 11 | `annuaire_incomplet` n'est plus rendu | les avertissements |
| 12 | Les obsolètes ne sont plus listés | `un_fichier_supprime_est_dit_supprime` |
| 13 | La comparaison de run devient toujours vraie (`même run` partout) | `un_deplacement_vers_un_autre_run_montre_les_deux` |
| 14 | La comparaison de run devient toujours fausse | `un_deplacement_qui_ne_change_pas_de_run_ne_repete_pas_la_valeur` |
| 15 | `destination` échappe son libellé (double échappement) | l'échappement |

- [ ] **Step 2: Combler chaque mutation survivante**

Une mutation qui laisse la suite verte est un test à écrire ou à durcir. Attention : une mutation peut être **équivalente** (le HTML produit est identique) — dans ce cas le noter en commentaire plutôt que d'inventer un test.

- [ ] **Step 3: Lancer les deux suites**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
cd ../ && node --test "tests/*.test.js"
```

Attendu : tout vert, et le compte de tests Rust supérieur à 607.

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/rapprochement_report.rs
git commit -m "test(superpopaul): durcit le rapport de rapprochement contre les mutations"
```

---

## Task 11: Vérification de bout en bout

- [ ] **Step 1: Les deux suites**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -30
cd ../ && node --test "tests/*.test.js"
```

- [ ] **Step 2: Comparer le rendu à la maquette**

Écrire un rapport de démonstration depuis un test temporaire (ou via l'application) et l'ouvrir à côté de `docs/superpowers/maquettes/2026-07-28-rapport-rapprochement.html`. Vérifier : l'alerte rouge n'apparaît que pour les retraits sur MEP transmise, les tableaux vides n'existent pas, le pied nomme le classeur.

- [ ] **Step 3: Ce qui ne se prouve qu'en application**

À signaler à l'utilisateur pour son parcours GUI, ces points n'étant atteignables par aucun test — `tauri::State` n'est pas constructible hors application montée :

- le rapport est bien écrit dans le répertoire de sortie configuré, sous son nom horodaté ;
- le bandeau nomme le rapport après application ;
- le verrou d'empreinte refuse toujours (rapprocher, modifier le fichier, appliquer) et **n'écrit aucun rapport** dans ce cas.

- [ ] **Step 4: Ne pas pousser**

Le push et la release restent demandés à chaque fois. S'arrêter ici et rendre compte.
