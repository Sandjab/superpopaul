# Plan de charge FUT (Runs de Facturation) — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Définir un plan de charge sur la base d'un calendrier de Runs de Facturation, l'appliquer au résultat d'une résolution complète, et produire la liste des comptes de facturation à activer MEP par MEP — avec retouche manuelle possible.

**Architecture:** Deux modules purs (`calendrier.rs`, `plan.rs`) testables sans UI, deux tables auto-porteuses (`plan_cf`, `plan_meta`) écrites transactionnellement, glue `*_from_scan` dans `commands.rs` (miroir de `coverage_from_scan` / `repartition_from_scan`), IHM à deux onglets sans logique métier.

**Tech Stack:** Rust (client Tauri), `cargo test` dans `client/src-tauri/`. **Aucune dépendance nouvelle** (`rusqlite`, `serde_yaml`, `sha2`, `chrono` déjà présents). Frontend vanilla, style « Bleu nuit & or ».

**Spec:** `docs/superpowers/specs/2026-07-25-plan-de-charge-fut-design.md`

**Maquette:** validée le 2026-07-25 (go explicite). Non versionnée, comme les précédentes.

---

## Découpage en deux vagues

| Vague | Tâches | Livrable | Point d'arrêt |
|---|---|---|---|
| **1 — Moteur** | 1 → 8 | Calcul complet + persistance, testable sans UI | `cargo test` vert, rien d'à moitié branché |
| **2 — Surface** | 9 → 13 | Commandes, fichiers, rapport, IHM | Fonctionnalité utilisable |

La vague 1 se termine sur un état cohérent : tout le métier est écrit et prouvé par les tests, aucune commande Tauri n'est exposée, aucun écran n'existe. C'est un point d'arrêt propre si le chantier doit être interrompu.

---

## File Structure

- **Create** `client/src-tauri/src/calendrier.rs` — Runs de Facturation, MEP, fenêtre, parsing `runs.csv`.
- **Create** `client/src-tauri/src/plan.rs` — funnel, pool, quotas, rampe, allocation, régénération, retouche.
- **Create** `client/src-tauri/src/plan_report.rs` — rapport HTML distinct (vague 2).
- **Modify** `client/src-tauri/src/csv_io.rs` — `read_columns`.
- **Modify** `client/src-tauri/src/config.rs` — `InputConfig` : `cf_column`, `jj_column`, `raison_sociale_column`.
- **Modify** `client/src-tauri/src/store.rs` — `plan_cf` / `plan_meta`, écriture transactionnelle, lectures.
- **Modify** `client/src-tauri/src/commands.rs` — `plan_*_from_scan` (glue) + commandes Tauri.
- **Modify** `client/src-tauri/src/lib.rs` — modules + enregistrement des commandes.
- **Modify** `client/src/index.html`, `app.js`, `styles.css` — écran à deux onglets.

**Note de convention (assumée) :** ce plan donne les **signatures, invariants et listes de tests**, pas le code intégral — le volume (~2 000 lignes de Rust) rendrait un plan exhaustif illisible et périmé dès la première divergence. Les extraits de code ne figurent que là où le point est subtil (socle de rampe, trois ensembles préservés, contrainte JJ). Comme pour `repartition_from_scan`, les fonctions glue `*_from_scan` ne sont **pas** testées unitairement : toute la logique testable vit dans les modules purs.

---

# VAGUE 1 — MOTEUR

## Task 1: Colonnes d'entrée (CF, JJ, raison sociale)

**Files:**
- Modify: `client/src-tauri/src/csv_io.rs`
- Modify: `client/src-tauri/src/config.rs`

- [ ] **Step 1: `csv_io::read_columns` (test-first)**

```rust
/// Lit plusieurs colonnes en UN seul passage. Erreur nommant la première
/// colonne absente de l'en-tête (même style de message que read_column).
pub fn read_columns(path: &Path, meta: &CsvMeta, columns: &[&str])
    -> Result<Vec<Vec<String>>, String>;
```

Tests : lecture de 3 colonnes dans l'ordre demandé ; colonne absente → erreur nommant la colonne et listant l'en-tête ; un seul passage (fichier lu une fois) ; colonne demandée deux fois → deux vecteurs identiques.

- [ ] **Step 2: `InputConfig` — trois champs optionnels rétro-compatibles**

`cf_column`, `jj_column`, `raison_sociale_column` : `#[serde(default, skip_serializing_if = "String::is_empty")]`. Un YAML d'avant reste lisible et n'est pas réécrit avec des champs vides.

Tests : désérialisation d'un YAML sans les champs → chaînes vides ; sérialisation d'une config sans mapping → champs absents du YAML ; aller-retour avec mapping renseigné.

**Verify:** `cargo test` vert. Aucun comportement existant modifié.

---

## Task 2: `calendrier.rs` — types et parsing `runs.csv`

**Files:**
- Create: `client/src-tauri/src/calendrier.rs`
- Modify: `client/src-tauri/src/lib.rs` (`pub mod calendrier;`, ordre alphabétique)

- [ ] **Step 1: types**

```rust
pub struct RunFacturation { pub num: String, pub date: NaiveDate,
                            pub jjs: Vec<u8>, pub exclu: bool }
impl RunFacturation { pub fn couvre(&self, jj: u8) -> bool }
```

- [ ] **Step 2: parsing (test-first)**

```rust
/// En-tête `DATE_RUN;NUM_RUN;JJS`. Date JJ/MM/AAAA stricte, JJ séparés par '-'.
/// Fail-loud ligne par ligne : TOUTES les erreurs sont collectées, pas d'arrêt
/// à la première. Messages actionnables, affichés tels quels.
pub fn parse_runs_csv(texte: &str) -> (Vec<RunFacturation>, Vec<String>);
```

Tests : nominal (runs triés par date, JJ triés/dédoublonnés) ; en-tête absent (première ligne = données) ; séparateur inattendu ; date `31/02/2026` inexistante ; date au format ISO → refusée ; JJ `0` et `32` ; JJ non numérique ; `NUM_RUN` vide ; numéro dupliqué ; deux runs à la même date ; **erreurs cumulées** (3 lignes fautives → 3 messages).

**Verify:** `cargo test` vert.

---

## Task 3: `calendrier.rs` — fenêtre, MEP, rattachement

**Files:** Modify: `client/src-tauri/src/calendrier.rs`

- [ ] **Step 1: runs utilisables (test-first)**

```rust
/// Non exclus, dans [debut, fin], STRICTEMENT postérieurs à la première MEP.
pub fn runs_utilisables(runs: &[RunFacturation], debut: NaiveDate,
                        fin: NaiveDate, meps: &[NaiveDate]) -> Vec<RunFacturation>;
```

Tests : run exclu écarté ; run hors bornes écarté (les deux bornes, incluses) ; **run le jour même d'une MEP écarté** (le « strictement » de la spec) ; sans MEP → liste vide.

- [ ] **Step 2: complétion des MEP (test-first)**

```rust
/// Dates fournies conservées ; complétion jusqu'à `voulu` par équirépartition
/// sur [debut, fin). Renvoie (meps triées dédoublonnées, avertissements).
pub fn completer_meps(runs: &[RunFacturation], debut: NaiveDate, fin: NaiveDate,
                      fournies: &[NaiveDate], voulu: usize)
    -> (Vec<NaiveDate>, Vec<String>);
```

Tests : équirépartition sur une fenêtre simple ; MEP fournies conservées telles quelles ; MEP **auto** sans run utilisable après elle → ramenée à la veille du dernier run candidat ; MEP **fournie** dans ce cas → conservée + avertissement ; fenêtre trop courte → avertissement « N MEP non planifiables » ; aucun run candidat → avertissement ; invariant `< fin` (jamais une MEP à la borne de fin).

- [ ] **Step 3: MEP de rattachement (test-first)**

```rust
/// Dernière MEP STRICTEMENT antérieure au run. `None` si aucune.
pub fn mep_de(run: NaiveDate, meps: &[NaiveDate]) -> Option<(usize, NaiveDate)>;
```

Tests : rattachement à la dernière antérieure (pas la première) ; run le jour d'une MEP → la précédente ; run avant toute MEP → `None` ; indice 1-basé cohérent avec `mep_id`.

**Verify:** `cargo test` vert. Module encore inutilisé par le reste.

---

## Task 4: `plan.rs` — pool éligible et funnel

**Files:**
- Create: `client/src-tauri/src/plan.rs`
- Modify: `client/src-tauri/src/lib.rs`

- [ ] **Step 1: types + agrégation pure (test-first)**

```rust
pub struct CfCandidat { pub cf: String, pub participant: String, pub jj: u8,
                        pub raison_sociale: String, pub pa: String,
                        pub in_directory: bool, pub resolved_at: i64 }

pub struct Funnel { pub lignes: u64, pub cf_distincts: u64, pub jj_valide: u64,
                    pub resolus: u64, pub ctc_ready: u64, pub ppf_usable: u64,
                    pub pa_exclue: u64, pub eligibles: u64 }

/// Entrées : une par LIGNE du CSV (cf, participant, jj brut, raison sociale)
/// + les jointures déjà faites (résolution, drapeaux PPF). PUR : aucune DB.
/// Erreur si un même CF porte des JJ divergents (décision 4 de la spec).
pub fn construire_pool(entrees: &[LigneEntree], exclues: &HashSet<String>,
                       maintenant: DateTime<Utc>)
    -> Result<(Vec<CfCandidat>, Funnel), String>;
```

Tests (chaque marche retire ce qu'elle doit **et rien d'autre**) :
- doublon strict de CF → dédoublonné en silence, `cf_distincts` décrémenté ;
- **même CF, JJ divergents → erreur nommant le CF et les deux JJ** ;
- JJ `0`, `32`, vide, non numérique → écartés à `jj_valide`, comptés ;
- adressage sans résolution en base → écarté à `resolus` ;
- `api_status != "ok"` → écarté à `resolus` ;
- CTC `later` et `expired` → écartés à `ctc_ready` ; `ready` → passe ;
- **`ppf_active` vrai mais `ppf_usable` faux → EXCLU** (le test qui encode la décision 1) ;
- PA exclue → écarté, `pa_exclue` compté ;
- `pa` via `repartition::pa_key` (nom, repli code) ;
- funnel monotone décroissant, `eligibles == pool.len()`.

- [ ] **Step 2: glue `plan_pool_from_scan` dans `commands.rs`**

Scan des 3–4 colonnes en un passage (`read_columns`), `load_map`, `ppf_flags(motifs globaux)`, puis `construire_pool`. Non testée unitairement (convention maison).

**Verify:** `cargo test` vert.

---

## Task 5: `plan.rs` — répartition, quotas, rampe

**Files:** Modify: `client/src-tauri/src/plan.rs`

- [ ] **Step 1: `plus_forts_restes` (test-first)**

Somme **exactement** égale au total ; départage déterministe (clé triée) ; total 0 ou poids nuls → tout à zéro.

- [ ] **Step 2: `quotas_par_pa` (test-first)**

Proportionnels au pool, **plancher 1** par PA ayant ≥ 1 éligible, **plafond = stock**, redistribution itérative de l'excédent.

Tests : plancher 1 respecté même pour une PA à 1 CF ; cible < nombre de PA → les mieux dotées servies d'abord ; plafond au stock avec redistribution ; somme des quotas = cible (ou = stock total si cible > stock).

- [ ] **Step 3: `construire_rampe` (test-first)**

Le point subtil est le **socle du pilote** — à porter tel quel :

```
Pilote actif (P premiers runs à V CF) :
  budget restant = cible − P·V
  si budget >= (runs restants)·V  → chaque run post-pilote démarre à V,
                                     la forme ne répartit que le SURPLUS
  sinon                           → socle abandonné, forme pure (creux sous V)
                                     + avertissement (rampe_pilote_infaisable)
```

Tests : les quatre formes (plate, linéaire, géométrique, manuelle) ; **somme = cible** dans tous les cas où runs non vide et cible > 0 ; socle tenu (aucun run post-pilote sous V) ; cible trop basse → forme pure + `rampe_pilote_infaisable` vrai ; pilote à V=0 ou P=0 → inerte, forme sur tous les runs ; P == nombre de runs → reliquat sur le dernier ; forme manuelle → volumes rendus verbatim, cible ignorée, run absent → 0.

**Verify:** `cargo test` vert.

---

## Task 6: `plan.rs` — allocation et couverture

**Files:** Modify: `client/src-tauri/src/plan.rs`

- [ ] **Step 1: tri de priorité (test-first)**

Clé composite unique, **pas de RNG** (décision 8) :

```rust
// (in_directory desc, resolved_at desc, fnv1a(seed, cf) asc)
```

Tests : `in_directory` prime sur la fraîcheur ; à priorité égale, l'ordre dépend du seed ; **même seed → même ordre** (déterminisme) ; seeds différents → ordres différents.

- [ ] **Step 2: allocation (test-first)**

```rust
pub struct LignePlan { /* cf, participant, jj, raison_sociale, pa, mep_id,
                          mep_date, run_num, run_date, origine, in_directory,
                          resolved_at */ }

pub fn allouer(pool: &[CfCandidat], runs: &[RunFacturation], meps: &[NaiveDate],
               seed: u64, cible: usize, rampe: &Rampe, preserves: &Preserves)
    -> (Vec<LignePlan>, Vec<String>);
```

Tests :
- volume non absorbable (stock insuffisant sur les JJ du run) → **glisse** au run suivant ;
- reliquat final → **avertissement**, pas une erreur ;
- aucun run utilisable et cible > 0 → avertissement « cible non atteinte » ;
- quotas **souples** : un run dont le volume dépasse les quotas restants complète tous PA confondus (comportement historique, décision 15) ;
- **couverture** : une PA non servie reçoit 1 CF `origine = 'couverture'` sur le PREMIER run chronologique couvrant le JJ d'un de ses candidats ;
- PA dont aucun candidat n'a de JJ couvert → avertissement nommant la PA ;
- un CF n'est jamais affecté deux fois.

**Verify:** `cargo test` vert.

---

## Task 7: `store.rs` — tables et écriture transactionnelle

**Files:** Modify: `client/src-tauri/src/store.rs`

- [ ] **Step 1: schéma**

`plan_cf` et `plan_meta` ajoutés au `SCHEMA` (colonnes exactes en spec). Points à ne pas manquer :
- `participant` en **forme stockée** (0225 nu, via `to_stored`) — joignable directement à `ppf_directory` sans `substr` ;
- `origine` TEXT (`auto` | `couverture` | `manuel`), pas de `coverage_fill` ;
- `retire_le` / `retire_motif` nullables ;
- `plan_meta` en table 1-ligne (`CHECK (id = 1)`), `params_yaml` sérialisé par `serde_yaml`.

- [ ] **Step 2: écriture transactionnelle (test-first)**

```rust
/// DROP + réinsertion + plan_meta dans UNE transaction (discipline de
/// replace_peppol_directory : l'horodatage ne peut pas diverger du contenu).
pub fn ecrire_plan(&self, lignes: &[LignePlan], meta: &PlanMeta) -> Result<(), String>;
pub fn charger_plan(&self) -> Result<Option<(Vec<LignePlan>, PlanMeta)>, String>;
```

Tests : aller-retour fidèle ; `participant` bien stocké nu et **relu en forme longue** ; échec en cours → aucune ligne écrite **et** meta inchangée ; base sans plan → `None` ; réécriture remplace intégralement.

**Verify:** `cargo test` vert.

---

## Task 8: Régénération et retouche manuelle

**Files:** Modify: `client/src-tauri/src/plan.rs`, `client/src-tauri/src/store.rs`

- [ ] **Step 1: les trois ensembles préservés (test-first)**

```rust
pub struct Preserves { pub gelees: Vec<LignePlan>,    // mep_date < date de gel
                       pub epinglees: Vec<LignePlan>, // origine == manuel
                       pub retirees: HashSet<String> } // cf retirés
```

Même mécanique pour les trois : leurs CF sortent du pool des candidats et consomment leur part de la cible.

Tests :
- une ligne `origine = 'manuel'` **survit à un changement de rampe** (le test qui empêche la perte silencieuse) ;
- une ligne `auto` est bien re-tirée ;
- **une ligne retirée n'est pas replacée** par la régénération suivante ;
- gelées + épinglées + retirées consomment leur part de la cible (**pas de double compte**) ;
- MEP gelée disparue de la configuration → **refus fort** avec message nommant la date.

- [ ] **Step 2: retouche manuelle — les cinq règles (test-first)**

```rust
pub fn runs_compatibles(jj: u8, runs: &[RunFacturation]) -> Vec<&RunFacturation>;
pub fn ajouter(/* … */) -> Result<Vec<LignePlan>, String>;
pub fn deplacer(/* … */) -> Result<(), String>;
pub fn retirer(cf: &[String], motif: &str, /* … */) -> Result<(), String>;
pub fn annuler_retrait(cf: &[String], /* … */) -> Result<(), String>;
```

Tests :
1. `runs_compatibles` ne contient que les runs couvrant le JJ ; `deplacer` vers un run incompatible → **erreur** (le sélecteur filtré est une commodité d'IHM, pas la garde) ;
2. ajout d'un CF **non éligible** → accepté, `origine = 'manuel'`, marqué ;
3. ajout d'un CF **absent du fichier courant** → refusé ;
4. `retirer` avec motif vide ou blanc → **refusé** ; retrait sur MEP gelée → **accepté et tracé** (`retire_le`, `retire_motif`) ; `annuler_retrait` → ligne redevient active ;
5. dépassement de cible par une retouche → accepté, signalé.

Toute ligne ajoutée ou déplacée passe en `origine = 'manuel'`.

**Verify:** `cargo test` vert. **Fin de vague 1** — tout le métier est écrit et prouvé, aucune commande exposée, aucun écran.

---

# VAGUE 2 — SURFACE

## Task 9: Commandes Tauri — calcul et persistance

**Files:** Modify: `client/src-tauri/src/commands.rs`, `lib.rs`

- [ ] `plan_import_runs(path)` → calendrier + erreurs de parsing.
- [ ] `plan_preview(params)` → funnel, pool, volumes, faisabilité. **Calcul réel** (pas une approximation), rien d'écrit — explorer des scénarios est gratuit.
- [ ] `plan_generate(params)` → `ecrire_plan` (1 transaction) + fichiers.
- [ ] `plan_load()` → état persisté au retour sur l'écran.
- [ ] `plan_status()` → CF du plan devenus inéligibles (recalcul, jamais figé).

Tout le corps bloquant (scan CSV, SQLite) via `spawn_blocking`, comme `generate_output`.

**Verify:** commandes enregistrées, `cargo test` vert, `cargo clippy` propre.

---

## Task 10: Commandes Tauri — retouche

**Files:** Modify: `client/src-tauri/src/commands.rs`, `lib.rs`

- [ ] `plan_lignes(filtres, tri)`, `plan_candidats(recherche)`, `plan_ajouter`, `plan_deplacer`, `plan_retirer`, `plan_annuler_retrait`, `plan_ecrire_fichiers`.

Les retouches sont écrites **immédiatement** (pas de brouillon) ; l'annulation passe par l'action inverse.

**Verify:** `cargo test` vert.

---

## Task 11: Fichiers livrables et rapport

**Files:** Create `client/src-tauri/src/plan_report.rs`; Modify `commands.rs`

- [ ] **Step 1: fichiers par MEP**

`<entrée>_plan_mep_<n>_<AAAA-MM-JJ>.txt` — CF nus, un par ligne, triés, **cumulatif** (MEP 1..n), UTF-8 sans BOM, `\n`. **Pas de manifest.** Les lignes retirées sont exclues, y compris sur MEP gelée.

Tests : cumul correct (MEP 2 contient MEP 1) ; tri stable ; ligne retirée absente de **tous** les fichiers ; répertoire résolu via `resolved_out_dir`.

- [ ] **Step 2: rapport `<entrée>_plan.html`**

KPI (planifiés, gelés, manuels, retirés, MEP, pool, PA couvertes), avertissements, table MEP/runs, répartition PA plan vs pool. Style et helpers de `report.rs`, `esc` sur toute donnée d'origine CSV ou SMP. Courbes → v2.

**Verify:** rapport généré et relu à l'œil.

---

## Task 12: IHM — panneau latéral et onglet 1

**Files:** Modify: `client/src/index.html`, `app.js`, `styles.css`

- [ ] Écran de plein niveau, deux onglets, layout deux colonnes.
- [ ] Panneau : bloc **Colonnes** (CF, JJ, raison sociale) éditable — c'est lui qui débloque l'écran quand le mapping manque, sans aller-retour vers l'étape 2 ; puis import `runs.csv`, fenêtre, MEP, cible, rampe, seed, action « Générer / Régénérer ».
- [ ] Onglet 1 : funnel (effectif **et** perte par marche), table des runs à **cinq colonnes chiffrées** (Visé · Report · Stock JJ · Placé · Reliquat), plateformes avec exclusion + quota et mention « cible souple », avertissements, bloc résultat.
- [ ] Point d'entrée « Établir un plan de charge → » en fin d'étape 3.

**Contraintes :** aucune logique métier dans l'UI (elle invoque, elle affiche) ; **jamais d'`innerHTML` avec des données dynamiques** — tout via le helper `h()` ; tokens `styles.css` uniquement.

**Verify:** validation GUI par l'utilisateur.

---

## Task 13: IHM — onglet 2 (comptes de facturation)

**Files:** Modify: `client/src/index.html`, `app.js`, `styles.css`

- [ ] Table filtrable (MEP, run, plateforme, origine, état, recherche) et triable sur toutes les colonnes.
- [ ] Sélection multiple + barre d'actions contextuelle (déplacer, retirer, désélectionner) — retoucher 40 CF un par un serait inutilisable.
- [ ] Modale d'ajout (recherche dans les CF hors plan, sélecteur de run **filtré par JJ**, CF non éligible ajoutable mais signalé).
- [ ] Modale de retrait : motif obligatoire (confirmation désactivée tant qu'il est vide), **avertissement fort si MEP gelée** annonçant le changement du fichier déjà transmis.
- [ ] Marquage des états, conforme à la maquette validée : gelé `--green-later`, manuelle `--pid`, couverture `--ppf-l3`, retiré `--red` + barré, devenu inéligible `--amber`. Ni or ni orange pour les quatre premiers.
- [ ] Compteur « N lignes actives · M retirées » : les retirées comptées **à part**, jamais fondues dans le total.

**Verify:** validation GUI par l'utilisateur, puis revue de code globale.

---

## Hors-scope (v2)

Timeline calendaire, fériés FR (computus de Meeus — décoratifs, vérifié : `extra_holidays` n'est lu par aucun calcul chez peppolstat), rampe manuelle saisie run par run, courbes du rapport (cumulée, charge par jour civil), export/import JSON du paramétrage.
