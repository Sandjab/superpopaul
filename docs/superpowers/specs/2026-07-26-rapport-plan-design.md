# Rapport de plan de charge — courbes et mise en page — design

Dernier lot de la v2 du plan de charge. Brainstorm du 2026-07-26, maquette
validée le même jour (`docs/superpowers/maquettes/2026-07-26-rapport-plan.html`).

## Contexte

Le rapport de plan (`plan_report.rs`, 265 lignes) est un livrable distinct du
rapport de run, généré par la commande `plan_rapport` (`commands.rs:1435`). Il
partage avec `report.rs` la constante `CSS` et les helpers `esc` / `fmt_int`.

**Constat vérifié à l'ouverture du chantier : ce rapport n'a jamais eu de
styles.** `plan_report::render` émet `class="cards"`, `class="card"`,
`class="big"`, `class="lbl"`, deux `<table>` et un `<ul>` ; or la constante
`CSS` (`report.rs:47`) ne définit **aucune** de ces classes, et ne contient
aucun sélecteur `table`, `th`, `td`, `section`, `ul` ni `li`. Le rapport de run,
lui, ne contient **zéro `<table>`** : toutes ses données passent par des grilles
CSS (`.pa-row`, `.cov-row`, `.sec-row`) et des cartes (`.kpis` / `.kpi`).

Conséquence à l'écran : les cartes d'indicateurs sont des `div` empilés sans
mise en forme, et les deux tableaux s'affichent avec le style par défaut du
navigateur sur un fond bleu nuit. Le ressenti rapporté par l'utilisateur — « la
présentation n'est pas parfaite » — ne décrit donc pas une finition à reprendre
mais une feuille de style jamais branchée.

Par ailleurs, l'en-tête du module annonce : *« Les courbes (cumulée, charge par
jour civil) sont hors périmètre v1 »*, et la spec de la v1
(`2026-07-25-plan-de-charge-fut-design.md`, § Rapport) les inscrit en v2. Elles
n'ont jamais été écrites.

## Objectif

Faire du rapport de plan un document unique lisible par ses quatre lecteurs —
l'auteur du plan qui le contrôle avant transmission, un comité de pilotage, les
équipes d'exploitation, les plateformes partenaires — en lui donnant :

1. les deux courbes prévues : parc facturant cumulé, et charge par run
   (premières factures + récurrences projetées) ;
2. la mise en page qui lui manque, alignée sur le rapport de run.

## Décisions validées (brainstorm 2026-07-26)

1. **Document unique, structuré en trois actes** — trajectoire, exécution,
   contrôle — plutôt que quatre sections adressées chacune à un lecteur, ce qui
   dupliquerait les mêmes chiffres. L'ordre retenu :

   | # | Section | Sert d'abord à |
   |---|---------|----------------|
   | ① | Indicateurs de trajectoire (4 cartes) | pilotage |
   | ② | Avertissements | contrôle |
   | ③ | Parc facturant (aire cumulée + jalons MEP) | pilotage |
   | ④ | Charge par run (barres empilées) | exploitation |
   | ⑤ | Mises en production et Runs (table) | tous |
   | ⑥ | Répartition par plateforme (grille à barres) | partenaires |
   | ⑦ | Contrôle du plan (4 cartes discrètes) | contrôle |

2. **Les avertissements remontent en position ②**, juste après les indicateurs.
   Ils étaient déjà hauts en v1 ; les laisser remonter plutôt que les reléguer
   en fin de document est délibéré — une alerte lue après les conclusions
   n'alerte plus.

3. **Récurrence : une facture par compte et par mois civil**, portée par le
   **premier run du mois** dont les jours de cycle couvrent le JJ du compte. Si
   deux runs du même mois couvrent le même JJ, le compte n'est compté qu'une
   fois. Écarté : la lecture littérale « tout run couvrant le JJ refacture »,
   qui produirait deux factures mensuelles pour un même compte.

4. **Horizon = les runs du `runs.csv`, sans extrapolation.** Tous les runs
   retenus sont représentés, **y compris ceux postérieurs à la dernière
   première facture** : c'est là que se lit le régime de croisière, et cela ne
   coûte aucune hypothèse. Écarté : prolonger le rythme des runs au-delà du
   fichier, qui aurait demandé d'inventer un calendrier.

5. **Les runs exclus n'apparaissent pas** dans le graphe de charge : un run
   écarté du plan est écarté du rapport, comme partout ailleurs dans ce
   livrable. Limite assumée et à connaître : si un run exclu tourne malgré tout
   en production, la charge affichée sous-estime la charge réelle.

6. **Neuf indicateurs, deux niveaux.** Le pool éligible devient une sous-ligne
   de « comptes planifiés » (classe `.kpi .abs`, déjà au CSS) plutôt qu'une
   carte, ce qui ramène à 4 cartes de trajectoire + 4 cartes de contrôle. Les
   quatre nouveaux indicateurs : part planifiée du pool, fin de montée en
   charge, pic de charge, plateformes couvertes.

7. **Deux graphes séparés**, pas un graphe combiné à double axe : un axe
   secondaire se prête aux erreurs de lecture et s'imprime mal. Écarté aussi :
   un troisième graphe des 31 jours de cycle — la distribution par JJ est déjà
   exposée dans l'écran Plan de charge (timeline), la répéter allongerait le
   rapport sans rien apprendre à ses lecteurs.

8. **Table conservée pour les MEP et Runs, grille à barres pour la
   répartition.** Sept colonnes de données franchement tabulaires justifient une
   `<table>` (et se copient vers un tableur) ; la comparaison plan / pool, elle,
   se lit d'un coup d'œil en barres et pas en colonnes de pourcentages.

9. **Découpage : deux modules neufs, le CSS reste où il est.** `charge.rs` pour
   le calcul, `charts.rs` pour les SVG, tous deux purs. Écarté : extraire la
   constante `CSS` vers un `report_css.rs`, qui aurait fait rouvrir un fichier
   que le chantier n'oblige pas à toucher.

### Décisions prises en dessinant la maquette, validées avec elle

10. **Le graphe cumulé est en escalier**, non lissé : le parc facturant saute à
    chaque run. Une courbe lissée suggérerait une progression quotidienne
    inexistante.
11. **L'axe du graphe de charge est catégoriel** — une barre par run,
    équidistantes. Limite assumée : les écarts de dates réels ne sont pas à
    l'échelle. Écarté : un axe en dates, plus juste mais moins lisible quand les
    runs se resserrent.
12. **Repère du pic** en pointillé sur le graphe de charge, qui rattache
    l'indicateur de tête à la figure.
13. **Badge « gelée »** dans la colonne MEP de la table : sans lui, le gel
    n'apparaît nulle part dans le corps du rapport.
14. **Écart en points** dans la répartition (`+4,4 pt` en or si sur-servie,
    `−1,6 pt` en ambre si sous-servie), pour rendre la comparaison actionnable.

### Décision prise pendant la rédaction du plan

15. **Les avertissements sont dérivés par le rapport lui-même.** Constat : le
    site d'appel passe `avertissements: &[]` (`commands.rs:1461`) et les
    avertissements produits par l'allocation ne sont **jamais persistés** dans
    `PlanMeta` — la section existe, elle est testée, et elle ne s'affiche
    jamais en production. Plutôt que styliser une boîte vide, `render` calcule
    ses propres avertissements à partir de ce qu'il a déjà :

    - une plateforme du pool sans **aucun** compte planifié ;
    - un jour de cycle du pool couvert par **aucun** run retenu — les comptes
      de ce JJ sont hors d'atteinte, quelle que soit la cible.

    Le champ `avertissements` de `PlanReportData` **disparaît** : un champ que
    seul un `&[]` alimente est un mensonge d'interface. Écarté : persister les
    avertissements de l'allocation dans `PlanMeta`, qui demanderait une
    migration et périmerait dès que le pool bouge, alors que le rapport
    recalcule justement sur des données fraîches. Écarté aussi :
    l'avertissement « fichier d'entrée différent », déjà porté par l'écran Plan
    de charge (`commands.rs:1128`).

## Architecture

Deux modules neufs, purs — aucune DB, aucune UI, aucun accès disque — dans la
lignée de `timeline.rs` :

```
plan_report.rs ──> charge.rs   (combien de factures à chaque run)
               └─> charts.rs   (SVG aire et barres empilées)
```

`plan_report.rs` reste le seul module à connaître la mise en page du rapport.
`charge.rs` ne connaît ni HTML ni SVG ; `charts.rs` ne connaît ni plan ni run —
il reçoit des séries de nombres et des étiquettes.

**Filtrage des runs exclus : à la charge de l'appelant.** `charge::charge`
reçoit les runs **déjà retenus** (par `calendrier::runs_utilisables`, comme le
reste du plan). Le module ne décide rien, conformément au contrat de pureté
retenu pour `timeline.rs`.

## Modèle de données

### `charge.rs`

```rust
pub struct ChargeRun {
    pub num: String,
    pub date: NaiveDate,
    /// Comptes dont la première facture tombe à ce run.
    pub premieres: usize,
    /// Comptes déjà en production qui refacturent à ce run.
    pub recurrences: usize,
}

/// `lignes` : lignes ACTIVES du plan (les retirées sont exclues par l'appelant,
/// comme partout dans ce rapport). `runs` : runs RETENUS, triés par date.
pub fn charge(lignes: &[LignePlan], runs: &[RunFacturation]) -> Vec<ChargeRun>;
```

Algorithme, en deux temps :

1. **Porteur du mois.** Pour chaque couple `(année-mois, jour de cycle)`,
   déterminer l'index du **premier** run de ce mois dont `jjs` contient ce jour
   de cycle. C'est ce run, et lui seul, qui portera la facture mensuelle des
   comptes ayant ce JJ.
2. **Comptage.** Pour chaque run d'index `i` :
   - `premieres` = comptes du plan placés sur ce run ;
   - `recurrences` = comptes dont le JJ a `i` pour porteur du mois de ce run
     **et** dont le run de démarrage est d'index strictement inférieur à `i`.

La condition « démarrage strictement antérieur » évite qu'une première facture
soit comptée deux fois : le mois du démarrage, le compte figure dans
`premieres` et jamais dans `recurrences`.

Cas particuliers que l'algorithme traite sans code dédié :
- un compte placé sur un run qui n'est pas le porteur de son mois (deux runs du
  mois couvrent son JJ) démarre bien à son run, puis récurre normalement au
  porteur des mois suivants ;
- un mois dont aucun run ne couvre un JJ donné ne produit **aucune** facture
  pour les comptes de ce JJ — trou assumé, fidèle au calendrier fourni ;
- aucun run, aucune ligne, ou un plan entièrement retiré → série vide.

### `charts.rs`

```rust
pub struct Barre { pub label: String, pub sous_label: String,
                   pub bas: u64, pub haut: u64 }
pub struct Point { pub date: NaiveDate, pub valeur: u64 }
pub struct JalonChart { pub date: NaiveDate, pub label: String }

/// Barres empilées (`bas` = premières factures, `haut` = récurrences).
/// Le repère du pic est tracé sur le maximum de la série : c'est sa
/// définition, il n'a pas à être passé en paramètre.
pub fn barres_empilees(barres: &[Barre]) -> String;

/// Aire cumulée en escalier, avec jalons verticaux.
pub fn aire_cumulee(points: &[Point], jalons: &[JalonChart],
                    debut: NaiveDate, fin: NaiveDate) -> String;
```

Les deux rendent un `<svg viewBox>` inline, sans JS ni dépendance, coloré par
les variables CSS existantes (`var(--gold)`, `var(--green-later)`) — le même
procédé que les anneaux de `report.rs`. L'échelle Y est arrondie au palier
supérieur lisible (500 / 1 000 / 5 000 …).

**`esc` est obligatoire sur les étiquettes de runs** : les numéros de run
viennent du `runs.csv`, donc d'une entrée non fiable, au même titre que les
données CSV et SMP.

### Définition des quatre indicateurs neufs

Toujours sur les lignes **actives** (les retirées sont exclues) :

| Indicateur | Définition | Cas dégénéré |
|------------|------------|--------------|
| Part planifiée du pool | `comptes actifs / somme(pool_par_pa)` | pool vide → `—`, jamais `0,0 %` (règle déjà tenue par `pourcent`) |
| Fin de montée en charge | date du **dernier run portant au moins une première facture**, et durée en mois pleins depuis la première MEP | plan vide → carte masquée |
| Pic de charge | `max(premieres + recurrences)` sur la série de `charge`, avec le numéro et la date du run | série vide → carte masquée |
| Plateformes couvertes | nombre de plateformes du **pool** ayant au moins un compte planifié, sur le nombre total de plateformes du pool | pool vide → `— / —` |

La fin de montée en charge est le dernier run **portant des premières
factures**, non le dernier run de la série : les runs de croisière qui suivent
sont bien affichés dans le graphe (décision 4) mais ne repoussent pas la date de
fin, sans quoi l'indicateur mesurerait la longueur du `runs.csv` et non celle du
déploiement.

### Données à ajouter à `PlanReportData`

Deux champs s'ajoutent, un disparaît :

```rust
pub runs: &'a [RunFacturation],          // AJOUT — les runs RETENUS
pub pool_par_jj: &'a BTreeMap<u8, usize>, // AJOUT — pool par jour de cycle
// pub avertissements: &'a [String],      // SUPPRIMÉ — cf. décision 15
```

Le site d'appel (`commands.rs:1435`) dispose déjà de `params`, du pool
recalculé et des lignes ; il obtient le calendrier par
`calendrier_du_plan(&meta)` (`commands.rs:1295`), qui rend précisément les runs
**utilisables** — c'est déjà ce que fait `plan_ajouter`. `pool_par_jj` se
construit sur place, comme `pool_par_pa`, en une passe sur le pool
(`CfCandidat` porte `jj` et `pa`). Les quatre indicateurs neufs et les deux
avertissements dérivés se calculent de là, sans nouvelle requête ni nouveau
stockage.

## Styles ajoutés au CSS partagé

Ajouts seuls, aucune règle existante modifiée — le rapport de run ne doit pas
bouger d'un pixel :

- `.kpis.sub` — bande d'indicateurs secondaire (contrôle) ;
- `.warn` — encadré d'avertissements, filet ambre à gauche ;
- `.chart`, `.axis`, `.grid`, `.tick`, `.area`, `.line`, `.mep`, `.b-first`,
  `.b-rec`, `.b-peak`, `.chart-legend` — les deux graphes ;
- `table`, `thead th`, `tbody td`, `.num`, `.tbl`, `.mep-cell`, `.frozen` — la
  table, aujourd'hui non stylée ;
- `.dist`, `.dist-row`, `.dist-bars`, `.dist-n`, `.dist-gap` — la répartition,
  qui réutilise `.bar` / `.bar i` déjà présents.

Le thème clair et l'impression sont pris en charge sans travail neuf : toutes
les couleurs passent par les variables déjà redéfinies sous
`prefers-color-scheme: light` et `@media print`.

## Tests (TDD, Rust)

### `charge.rs`

1. `un_compte_ne_facture_qu_une_fois_par_mois` — deux runs du même mois couvrant
   le même JJ : le compte compte pour un, au premier des deux. **C'est le test
   qui distingue la règle retenue de la lecture littérale écartée.**
2. `pas_de_recurrence_avant_le_demarrage` — au run de démarrage, le compte est
   dans `premieres` et absent de `recurrences`.
3. `mois_sans_run_couvrant_le_jj_ne_facture_pas` — trou assumé, pas de report
   silencieux sur le mois suivant.
4. `les_runs_sans_premiere_facture_portent_les_recurrences` — un run postérieur
   à la dernière MEP a `premieres == 0` et `recurrences > 0` ; c'est le régime
   de croisière, il ne doit pas disparaître de la série.
5. `un_compte_place_hors_porteur_recurre_quand_meme` — le compte démarré au
   second run du mois récurre au porteur du mois suivant.
6. `serie_vide_sans_run_ni_ligne` — aucun panic, aucune division par zéro.

### `charts.rs`

7. `echelle_arrondie_au_palier_superieur`.
8. `serie_vide_rend_un_svg_valide_sans_paniquer`.
9. `valeurs_toutes_nulles_ne_divisent_pas_par_zero`.
10. `les_etiquettes_de_run_sont_echappees` — un run nommé `<script>` ne doit pas
    ressortir tel quel : le `runs.csv` est une entrée non fiable.

### `plan_report.rs`

11. Les sections neuves sont présentes (graphes, table stylée, deux bandes
    d'indicateurs).
12. Les quatre indicateurs neufs affichent les valeurs attendues.
13. Les tests d'échappement existants restent verts.
14. `avertit_sur_une_plateforme_du_pool_sans_compte_planifie` — une PA présente
    au pool et absente du plan produit un avertissement nommant la PA.
15. `avertit_sur_un_jour_de_cycle_hors_datteinte` — un JJ du pool qu'aucun run
    retenu ne couvre produit un avertissement nommant le JJ et l'effectif.
16. `aucun_avertissement_quand_le_plan_couvre_tout` — pas de section
    Avertissements dans le HTML, plutôt qu'une section vide.
17. `les_avertissements_derives_sont_echappes` — le nom de plateforme vient
    d'un SMP : `<script>` ne doit pas ressortir tel quel.

### Passe de mutation

Obligatoire sur tout le code neuf, avant de déclarer le lot fini. La session du
25–26/07 a trouvé un test incapable d'échouer à **chacune** de ses cinq tâches
Rust ; la règle est désormais systématique, pas discrétionnaire.

### Parcours GUI

Ouverture réelle du rapport depuis l'application — les trois défauts de la
session précédente étaient invisibles hors application. À vérifier : thème
sombre, thème clair, aperçu d'impression.

## Hors-scope

- **Projection au-delà du `runs.csv`** (décision 4).
- **Graphe des 31 jours de cycle** dans le rapport (décision 7).
- **Extraction de la constante `CSS`** vers un module dédié (décision 9).
- **`plan_meta.hash` jamais comparé** — l'avertissement « fichier différent »
  compare le nom (`commands.rs:1128`) alors que le SHA-256 du contenu est
  enregistré. Suivi ouvert, jamais tranché, sans rapport avec ce lot.
- **CI `actions/*@v4` → v5** (avertissement Node 20). Maintenance pure.

## Fichiers touchés

| Fichier | Nature |
|---------|--------|
| `client/src-tauri/src/charge.rs` | **neuf** — calcul des factures par run |
| `client/src-tauri/src/charts.rs` | **neuf** — SVG aire et barres |
| `client/src-tauri/src/plan_report.rs` | refonte du rendu, `PlanReportData` étendu |
| `client/src-tauri/src/report.rs` | ajouts au `CSS` uniquement |
| `client/src-tauri/src/lib.rs` | déclaration des deux modules |
| `client/src-tauri/src/commands.rs` | `plan_rapport` fournit les runs retenus |
| `docs/superpowers/maquettes/2026-07-26-rapport-plan.html` | maquette validée |
