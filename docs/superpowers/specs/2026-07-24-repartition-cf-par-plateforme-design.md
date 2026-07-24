# Répartition des CF par plateforme (PA) dans le rapport — design

Date : 2026-07-24

## Contexte

Le rapport HTML de fin de run (`report.rs`, livrable client) contient déjà :
- un **ring** « Répartition des adressages » (résolu / CTC / non résolu), en SVG statique ;
- une section **« Plateformes de dématérialisation constatées · en adressages uniques »**
  (liste en barres), alimentée par `snapshot.pa` (télémétrie du run : adressages par PA).

L'utilisateur veut une **vue en lignes/CF** (pas en adressages uniques) : la répartition
des lignes du fichier par Point d'Accès (PA), avec un ring top 5 + « autres » et la liste
détaillée de toutes les plateformes découvertes.

## Objectif

Ajouter au rapport une **nouvelle section** « Répartition des {record_plural} par plateforme »
(p. ex. « … par plateforme · en CF ») :
- un **ring** (donut) : 5 plateformes les plus fréquentes + « autres » (agrégé) + « sans plateforme » ;
- une **liste détaillée** de **toutes** les plateformes, triée par nombre de {record} décroissant,
  avec barre, effectif et pourcentage à **2 décimales**.

La section existante « en adressages uniques » reste **inchangée**.

## Décisions validées (maquettes companion, 2026-07-24)

1. **Ring en base « toutes les lignes du fichier »** : il inclut un segment « sans plateforme »
   (lignes non résolues ou résolues sans PA) ; les segments totalisent 100 % du fichier.
2. **Liste détaillée sans** la catégorie « sans plateforme » (uniquement les plateformes).
3. **Une seule base de %** partout = **total des lignes du fichier**. La liste omet la *ligne*
   « sans plateforme » mais ses % restent sur le total ; leur somme est donc < 100 %
   (le solde = les sans‑plateforme, montrés dans le ring). Objectif : un même PA n'a jamais
   deux pourcentages différents entre le ring et la liste.
4. **Regroupement par nom de PA** (`pa_name`), repli sur `pa_code` si le nom est vide.
5. **Barres colorées par plateforme** (top 5 = couleurs du ring, reste en teinte neutre).
6. **Nouvelle** section, placée **juste après** « Plateformes … · en adressages uniques ».

## Architecture

Calcul **au moment de l'export**, greffé sur le scan déjà effectué par `export_report`
(celui qui alimente `coverage` / `securisation`). Pattern maison identique à
`coverage_from_scan` / `securisation_from_scan`.

- Nouveau module `repartition.rs` (logique métier, testable sans UI).
- `export_report` (commands.rs) : appelle la nouvelle fonction sur le scan existant
  (mêmes `pids` + `line_counts`, même `store.lock()`), et passe le résultat à `ReportData`.
- `ReportData` (report.rs) gagne un champ `repartition_pa: Option<&Repartition>`
  (None → section non rendue, comme `securisation`).

Justification du choix (vs télémétrie) : le `snapshot` n'expose que « adressages par PA »
et des compteurs de lignes **globaux** — pas le croisement lignes×PA. L'étendre alourdirait
le chemin chaud du run. Conséquence assumée : la section porte sur le **fichier courant ×
résolutions en base** (comme `coverage`/`securisation`), et non sur le run.

## Modèle de données

```rust
// repartition.rs
pub struct PaCount { pub nom: String, pub lignes: u64 }

pub struct Repartition {
    pub total_lignes: u64,       // dénominateur des % (toutes les lignes du fichier)
    pub plateformes: Vec<PaCount>, // toutes les PA, triées lignes décroissant
    pub sans_plateforme: u64,    // lignes non résolues OU résolues sans PA
}

/// `pids` canoniques + `line_counts` issus du scan ; résolutions lues via load_map.
pub fn from_scan(store: &Store, pids: &[String], line_counts: &HashMap<String,u64>)
    -> Result<Repartition, String>;
```

Règles d'agrégation :
- Poids de chaque adressage = `line_counts[pid]` (multiplicité des lignes/CF ; défaut 1 si absent).
- Clé PA = `pa_name` non vide, sinon `pa_code` non vide, sinon **aucune** → `sans_plateforme`.
- Adressage sans résolution en base → `sans_plateforme`.
- `plateformes` triées par `lignes` décroissant (départage stable par nom, comme `telemetry::ranked`).
- `total_lignes` = somme de toutes les lignes (plateformes + sans_plateforme) = total du fichier scanné.

## Rendu (`report.rs`)

Nouvelle fonction `repartition_section(&mut html, &Repartition, record_plural)`, appelée
juste après la section « Plateformes … · en adressages uniques ». Rendue seulement si
`total_lignes > 0` et au moins une plateforme.

- **Titre** : `<h2>Répartition des {record_plural} par plateforme <span class="unit">· en {record_plural}</span></h2>`.
- **Ring** (SVG statique, comme le ring existant) : segments = top 5 plateformes +
  « autres » (somme des plateformes de rang ≥ 6, si non vide) + « sans plateforme » (si > 0).
  Centre = `fmt_int(total_lignes)` + libellé. Légende à droite (nom, effectif, % à 2 décimales).
- **Liste** : `.pa-row` (nom · barre · effectif · %) pour **toutes** les plateformes,
  largeur de barre relative au max, barre colorée (top 5 = couleur de segment, reste neutre).
- **%** : helper 2 décimales, base `total_lignes` (locale FR : « 32,50 % »).
- Nom de PA **échappé** (`esc`) — valeur issue des SMP, entrée non fiable (invariant du rapport).

Palette du ring/barres (catégorielle, sur fond « Bleu nuit ») : réutiliser les tokens
existants (`--gold`, `--green`, `--ppf-l3`, `--amber`) + une teinte bleue et une neutre
(`--track`/gris) pour « autres » ; gris foncé pour « sans plateforme ». Détail figé à
l'implémentation (maquette validée : #d9a83f, #4cc268, #a892ff, #e0873a, #5aa9e6, #6b7794, #3a4460).

## Tests (TDD, Rust)

Module `repartition::tests` :
- pondération par `line_counts` (une PA avec des adressages de multiplicités variées) ;
- regroupement par `pa_name` ; repli sur `pa_code` quand `pa_name` vide ;
- adressage sans résolution → `sans_plateforme` ; résolu sans pa_code ni pa_name → `sans_plateforme` ;
- tri décroissant + départage stable par nom à effectif égal ;
- `total_lignes` = plateformes + sans_plateforme ;
- traversée > 500 pids (lots load_map).

Rendu : si `report.rs` a des tests, ajouter un test de présence de la section et du % 2 décimales ;
sinon, validation visuelle via maquette déjà faite + relecture du HTML généré.

## Hors-scope

- La section « en adressages uniques » (snapshot) : inchangée.
- La télémétrie / le chemin de run : aucun changement.
- Les en-têtes du CSV de sortie (`field_name`) : inchangés.
- Parité CLI : non concernée (le rapport est propre au client Tauri).

## Fichiers touchés

- `client/src-tauri/src/repartition.rs` (nouveau) + déclaration du module.
- `client/src-tauri/src/report.rs` : champ `ReportData.repartition_pa`, `repartition_section`,
  éventuels tokens CSS.
- `client/src-tauri/src/commands.rs` : `export_report` calcule et passe la répartition.
