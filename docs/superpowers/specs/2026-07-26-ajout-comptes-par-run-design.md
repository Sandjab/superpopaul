# Ajout de comptes par run, superposition, export XLSX — design

Chantier issu du parcours GUI du 2026-07-26 : le rapport de plan a été validé,
l'**ajout manuel de comptes** ne l'a pas été. Brainstorm et maquette validée le
même jour (`docs/superpowers/maquettes/2026-07-26-ajout-comptes-par-run.html`).

Trois volets indépendants, livrables séparément.

## Contexte

### Volet A — la fenêtre d'ajout passe sous l'écran du plan

`#plan-screen` est un écran plein en `z-index: 60` (`styles.css:470`) ; le
conteneur de modale `#modal-backdrop` est en `z-index: 50` (`styles.css:377`).
Toute modale ouverte depuis l'écran Plan de charge est donc **recouverte** : il
faut la fermer pour voir ce qu'elle affichait. Constaté en application.

### Volet B — le point d'entrée de l'ajout est le mauvais

Le flux actuel part des **comptes** : `plan_candidats` (`commands.rs:1234`) rend
tous les comptes du fichier absents du plan, l'utilisateur en choisit, puis
`plan_runs_compatibles` calcule l'intersection des runs acceptant leurs jours de
cycle (`app.js:1995-2005`). Le run n'est qu'une conséquence.

Or la décision réelle part du **run** : on choisit un Run de Facturation, puis on
décide quels comptes y placer. De plus, la liste actuelle n'affiche que
`{ cf, raison_sociale, jj, pa, eligible }` — le statut CTC et l'utilisabilité PPF
sont **aplatis** en un seul booléen par `etat_de` (`commands.rs:1169`), alors que
ce sont précisément les informations sur lesquelles on arbitre. Aucun tri, aucun
filtre : seulement une recherche textuelle.

### Volet C — aucun tableau du périmètre complet

Le plan produit des `.txt` par MEP (CF nus) et un rapport HTML. Rien ne donne,
en une table exploitable, l'état de **tous** les comptes du fichier d'entrée :
lesquels sont au plan, sur quel run, et pourquoi les autres n'y sont pas.

## Objectif

1. Rendre la fenêtre d'ajout visible depuis l'écran du plan.
2. Faire du **run** le point d'entrée de l'ajout, avec une liste triable et
   filtrable portant les informations de décision.
3. Produire, avec les fichiers de livraison, un **classeur XLSX** couvrant
   l'intégralité des comptes du fichier d'entrée.

## Décisions validées (brainstorm 2026-07-26)

1. **Point d'entrée : la timeline, un run à la fois.** Chaque Run de Facturation
   **retenu** porte une action « + Ajouter ». Le run est donc déjà désigné à
   l'ouverture de la fenêtre. Écarté : conserver le bouton global avec un choix
   de run en première étape (une étape de plus pour le même résultat), et
   maintenir les deux entrées (un chemin de plus à tester pour un seul geste).
2. **Le bouton global « + Ajouter des comptes… » disparaît** de l'onglet
   *Comptes de facturation*. Conséquence assumée de la décision 1 : deux points
   d'entrée pour un même geste finissent par diverger.
3. **Filtre dur sur le jour de cycle, statuts affichés.** La fenêtre liste les
   comptes absents du plan dont le JJ est couvert par ce run — contrainte
   arithmétique, un run ne peut pas facturer un autre jour. Les comptes **CTC
   non prêt** ou **PPF non utilisable** restent listés et **signalés** : les
   forcer demeure un choix assumé, comme aujourd'hui. Écarté : ne lister que les
   pleinement éligibles, qui priverait du forçage d'un pilote prêt côté PDP et
   viderait de sens les colonnes de statut.
4. **Un run écarté ne porte pas l'action** : on ne peut rien y placer.
5. **XLSX produit par « Générer le plan »**, à côté des `.txt`, dans le même
   dossier. C'est le geste de livraison : ce qu'on transmet et ce qui documente
   le périmètre partent ensemble et restent cohérents. Écarté : le produire avec
   le rapport HTML (on pourrait alors livrer sans jamais produire le tableau).
6. **Une ligne par compte du fichier d'entrée**, pas du pool — donc y compris
   les comptes non résolus, sans plateforme identifiée ou écartés des quotas.
   L'unicité est garantie en amont : les lignes strictement identiques sont
   fondues et un même CF sur deux jours de cycle provoque un **refus fort**
   (`plan.rs:70-74`).
7. **Comptes retirés : colonne à trois valeurs, run conservé.** « Dans le plan »
   vaut `oui` / `retiré` / `non`, et le n° de run reste renseigné pour un
   retiré. Sans cela un retrait est indiscernable d'un compte jamais placé,
   alors que ce sont deux décisions opposées.
8. **Adressage sous forme stockée, sans ICD** (`552100554`) — exactement ce que
   rend `directory::parse_0225_value`. Un adressage dont le schéma n'est pas
   0225 sort sous sa forme canonique complète, faute de valeur nue à extraire.

   *Amendé le 26/07 après implémentation.* La rédaction initiale disait « forme
   nue » mais l'illustrait par `0225:12345678900012`, ce qui n'est pas la même
   chose : `parse_0225_value` retire l'ICD. L'implémenteur a signalé la
   contradiction plutôt que de choisir seul. L'utilisateur a tranché pour la
   forme réellement stockée, au motif que le CSV de sortie et la base l'écrivent
   ainsi depuis le 24/07 — le classeur se recoupe donc avec les autres exports
   et avec l'annuaire PPF sans retraitement.
9. **Statuts CTC en valeurs brutes** (`ready`, `later`, `expired`, vide), dans le
   fichier **et** dans la fenêtre d'ajout. **Écart assumé** à la règle du projet
   « texte UI en français » (`CLAUDE.md`) : tranché explicitement par
   l'utilisateur, au motif que la même valeur circule alors dans le fichier,
   l'IHM et les autres sorties, et que les filtres se transposent d'un artefact
   à l'autre. À ne pas « corriger » sans le rouvrir.
10. **Vrai `.xlsx` via `rust_xlsxwriter`** (crate Rust pure, sans dépendance C).
    Justifié par l'en-tête figé et les filtres automatiques natifs, qu'un CSV ne
    peut pas porter — et qui sont l'usage attendu du fichier. Écarté : CSV
    point-virgule (Excel massacre les identifiants longs en notation
    scientifique) et SpreadsheetML (avertissement d'extension à l'ouverture).

### Décisions prises en dessinant la maquette, validées avec elle

11. **Variante `.modal-wide`** plutôt qu'élargissement de `#modal`, qui plafonne
    à 460 px : l'élargir globalement déformerait toutes les confirmations.
12. **En-tête de tableau collant** dans la liste : sur plus de cent comptes, on
    perd sinon le sens des colonnes, et donc la lecture du tri en cours.
13. **Bandeau rappelant le run** (numéro, date, jours de cycle couverts, MEP de
    rattachement) : la fenêtre ne propose plus de choisir, il faut voir sans
    ambiguïté à quoi on ajoute.
14. **Non éligibles atténués et marqués ⚠, jamais barrés** — barrer signifierait
    « impossible » alors qu'ils restent sélectionnables. Le pied de fenêtre
    compte séparément les non éligibles présents dans la sélection.
15. **Filtres à plat sur une ligne, jamais repliés** : un filtre replié est un
    filtre oublié, et ce qui restreint la liste doit se voir d'un coup d'œil.

## Architecture

### Volet A

Une règle CSS. `#modal-backdrop` passe en `z-index: 70` : au-dessus de
`#plan-screen` (60), sous `#splash` (99). `#settings-backdrop` (40) reste sous
la modale, ce que son commentaire exige explicitement pour que la saisie des
identifiants proxy s'empile par-dessus.

### Volet B

```
timeline (app.js)  ──► plan_candidats_run(run_num)  ──► fenêtre de choix
                             │                              │
                             └─ filtre JJ côté Rust         └─ tri + filtres côté JS
```

- **`plan_candidats_run(run_num: String) -> Vec<Candidat>`** remplace
  `plan_candidats`, qui perd son unique appelant et **est supprimée** (y compris
  de l'`invoke_handler`) : une commande sans appelant est du code mort.
- Le **filtrage par jour de cycle se fait en Rust**, où le run est connu et où
  la règle `RunFacturation::couvre` existe déjà. Le JS ne refiltre pas sur le
  JJ : une seule autorité.
- **Tri et filtres se font en JS**, sur des données déjà en mémoire — un
  aller-retour par frappe de clavier serait absurde.
- `plan_runs_compatibles` **reste** : elle sert aussi au déplacement
  (`app.js:1914`), qui n'est pas dans le périmètre de ce chantier.

### Volet C

```
plan_generate ──► plan_xlsx::lignes(entrees, lignes_du_plan)  (PUR, testable)
                        │
                        └──► plan_xlsx::ecrire(chemin, &lignes)  (I/O, rust_xlsxwriter)
```

Le calcul est séparé de l'écriture : la composition du tableau se teste sans
toucher au disque ni au format, et le rendu XLSX n'a aucune logique métier.
C'est la même séparation que `charge` / `charts` dans le lot précédent.

## Modèle de données

### `Candidat` enrichi (`commands.rs:1161`)

```rust
pub struct Candidat {
    pub cf: String,
    pub raison_sociale: String,
    pub jj: u8,
    pub pa: String,
    pub eligible: bool,
    pub participant: String,   // AJOUT — adressage nu (cf. décision 8)
    pub ctc_status: String,    // AJOUT — "ready" | "later" | "expired" | ""
    pub ppf_usable: bool,      // AJOUT
}
```

`eligible` est conservé : il reste l'agrégat qui décide du marquage ⚠.

### `LigneEntree` (`plan.rs:21`)

Un champ s'ajoute :

```rust
pub ctc_status: String,   // AJOUT — "ready" | "later" | "expired" | ""
```

Le calcul existe déjà : `commands.rs:816` évalue
`output::ctc_status(r, now) == "ready"` pour en tirer `ctc_ready`. On conserve la
chaîne au lieu de la jeter. `ctc_ready` **reste** — il est consommé par
`construire_pool` et par le funnel d'éligibilité ; le dériver partout de
`ctc_status` élargirait le chantier sans bénéfice.

### `plan_xlsx.rs` (neuf)

```rust
pub enum Appartenance { Oui, Retire, Non }

pub struct LigneExport {
    pub run: String,              // vide si jamais placé ; conservé si retiré
    pub cf: String,
    pub jj: String,               // jj_brut : ce que contenait le fichier
    pub adressage: String,        // forme nue (cf. décision 8)
    pub raison_sociale: String,
    pub ctc_status: String,
    pub ppf_usable: bool,
    pub appartenance: Appartenance,
}

/// Compose le tableau. PUR : ni disque, ni format.
pub fn lignes(entrees: &[LigneEntree], plan: &[LignePlan]) -> Vec<LigneExport>;

/// Écrit le classeur : en-tête figé, filtres automatiques, largeurs.
pub fn ecrire(chemin: &Path, lignes: &[LigneExport]) -> Result<(), String>;
```

Nom du fichier : `<souche>_plan_comptes.xlsx`, à côté des
`<souche>_plan_mep_<n>_<date>.txt`.

Colonnes, dans cet ordre : **N° de run · N° de CF · JJ · Adressage · Raison
sociale · Statut CTC · PPF usable · Dans le plan**.

## IHM

### Timeline (onglet Paramétrage)

Une colonne d'action en fin de ligne. Seules les lignes `tr.tl-run` **non
écartées** portent le bouton « + Ajouter » (`.tl-add-btn`), discret au repos et
doré au survol de la ligne.

### Fenêtre d'ajout

`#modal.modal-wide`, en trois zones :

- **tête** : titre, bandeau du run (numéro, date, jours de cycle couverts, MEP),
  phrase d'explication, barre de filtres (recherche libre, plateforme, CTC,
  PPF, « réinitialiser ») ;
- **corps** : table `.plan-data` défilante, en-tête collant, colonnes triables
  via `th.sortable` / `th.sorted` — mécanisme **déjà écrit** pour l'onglet des
  comptes (`styles.css:535-536`), réutilisé et non réinventé ;
- **pied** : compte des sélectionnés, dont les non éligibles, puis « Annuler » et
  « Ajouter au run *N* ».

Les statuts sont rendus en pastilles (`.st-ready`, `.st-later`, `.st-expired`,
`.st-none`, `.st-yes`, `.st-no`) reprenant la sémantique de couleur du projet :
vert pour prêt, vert éteint pour « plus tard », rouge pour expiré, violet pour
la famille PPF. Jamais l'or (réservé à l'action) ni l'orange (avertissement).

## Tests

### Volet A (JS, `client/tests/`)

1. `la_modale_passe_au_dessus_de_l_ecran_plan` — lit `styles.css` et vérifie
   l'**ordre** des couches : `settings < modale < plan-screen` est faux, on
   attend `settings < plan-screen < modale < splash`. Porter sur l'ordre et non
   sur les valeurs : un test sur `z-index: 70` casserait au prochain réglage
   sans rien signaler d'utile.

### Volet B (Rust)

2. `candidats_run_ne_rend_que_les_jours_de_cycle_couverts` — un compte de JJ 12
   est absent pour un run couvrant `1, 5`.
3. `candidats_run_exclut_les_comptes_deja_au_plan`.
4. `candidats_run_rend_les_non_eligibles_signales` — un compte `ctc_status`
   valant `later` est présent, avec `eligible == false`.
5. `candidats_run_porte_le_statut_ctc_complet` — `later` et `expired` ne sont pas
   aplatis en `false` ; **c'est le test qui distingue le champ neuf du booléen
   préexistant**.
6. `candidats_run_refuse_un_run_inconnu` — erreur nommant le run, pas une liste
   vide qui ferait croire à une absence de candidats.

### Volet B (JS)

7. `le_tri_par_colonne_reordonne_la_liste`.
8. `les_filtres_se_combinent` — plateforme **et** CTC actifs simultanément.
9. `reinitialiser_restaure_la_liste_complete`.

### Volet C (Rust)

10. `export_couvre_toutes_les_lignes_du_fichier` — un compte non résolu, absent
    du pool, figure au tableau avec « non ».
11. `un_compte_retire_conserve_son_run_et_vaut_retire` — décision 7.
12. `un_compte_jamais_place_a_un_run_vide_et_vaut_non`.
13. `l_adressage_sort_sous_forme_nue` — `0225:123…` et non
    `iso6523-actorid-upis::0225:123…`.
14. `un_adressage_non_0225_sort_sous_forme_canonique` — le repli de la
    décision 8.
15. `le_statut_ctc_nest_pas_aplati` — `later` sort `later`, pas vide.
16. `le_classeur_porte_ses_filtres_et_son_volet_fige` — **ajouté le 26/07**.
    L'en-tête figé et les filtres automatiques sont ce qui justifie un vrai
    `.xlsx` plutôt qu'un CSV ; sans ce test, les retirer ne cassait rien jusqu'à
    ce que quelqu'un ouvre le fichier. Vérifié en décompressant
    `xl/worksheets/sheet1.xml`, via `zip` en `[dev-dependencies]` — crate déjà
    tiré en transitif par `rust_xlsxwriter`, donc aucun téléchargement neuf.

### Passe de mutation

Obligatoire sur tout le code neuf. **Les mutations sont dérivées du code écrit,
jamais du plan** : lors du lot précédent, les mutations imaginées depuis le plan
visaient ce qu'on croyait avoir testé, et un audit lisant le code en a trouvé
huit qui survivaient toutes.

Piège spécifique à ce projet, vérifié quinze fois lors du lot précédent : **une
chaîne cherchée dans un document riche a presque toujours plus d'un
producteur**. Vérifier, pour chaque assertion, que la chaîne visée n'a qu'une
source possible.

### Parcours GUI

Ouvrir la fenêtre depuis un run, trier, filtrer, ajouter — dont un compte non
éligible. Vérifier que le classeur s'ouvre dans Excel avec ses filtres.

## Hors-scope

- **Le déplacement de comptes** (`ouvrirDeplacer`) garde son flux actuel : on
  part de la sélection, les runs compatibles s'en déduisent. Le retourner n'a
  pas été demandé.
- **Le motif de retrait** dans le classeur (décision 7, variante écartée).
- **`plan_meta.hash` jamais comparé** — suivi ouvert de longue date.
- **Les 3 tests de `report.rs` cherchant une sous-chaîne dans le HTML entier**
  (`~997/1008/1016`), dette identifiée au lot précédent.
- **CI `actions/*@v4 → v5`.**

## Fichiers touchés

| Fichier | Nature |
|---------|--------|
| `client/src/styles.css` | `z-index` de la modale, `.modal-wide`, styles de la liste et de l'action timeline |
| `client/src/app.js` | action par run, fenêtre de choix (tri/filtres), suppression du bouton global |
| `client/src-tauri/src/plan_xlsx.rs` | **neuf** — composition et écriture du classeur |
| `client/src-tauri/src/commands.rs` | `plan_candidats_run`, suppression de `plan_candidats`, `Candidat` enrichi, `LigneEntree.ctc_status`, branchement dans `plan_generate` |
| `client/src-tauri/src/plan.rs` | `LigneEntree.ctc_status` |
| `client/src-tauri/src/lib.rs` | module `plan_xlsx`, `invoke_handler` |
| `client/src-tauri/Cargo.toml` | dépendance `rust_xlsxwriter` |
| `client/tests/` | tests JS des volets A et B |
