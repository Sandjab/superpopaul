# Présence en annuaire — cockpit + rapport HTML

Spec de conception. Date : 2026-07-19.

## Objectif

Faire apparaître dans le **cockpit** (étape 3 « Run ») et dans le **rapport HTML** la
couverture des identifiants d'entrée par les annuaires chargés, c.-à-d. les 5 notions
déclaratives déjà produites en sortie CSV mais aujourd'hui absentes de toute vue de synthèse :

- **Annuaire Peppol** — `in_directory` : présence déclarative dans l'annuaire Peppol chargé.
- **Annuaire PPF** (4 drapeaux `PpfFlags`) :
  - `annuaire_ppf` (`in_ppf`) — présent dans l'annuaire PPF (≥ 1 ligne).
  - `ppf_active` (`active`) — ≥ 1 ligne au motif ∈ {C, P}.
  - `pdp_definie` (`pdp_definie`) — ≥ 1 ligne avec PDP **réelle** (`UTILISE_PDP_FICTIVE = 0`).
  - `ppf_usable` (`usable`) — ≥ 1 **même** ligne : motif ∈ {C,P} **et** PDP réelle.

## Constat de départ (existant vérifié)

- Le « dashboard » = le **cockpit** de l'étape 3 (`index.html:137`, piloté par `cockpit.js`).
  Bandeau métier (`#biz-band`) = anneau global + 2 tuiles (« Provisionnés Peppol », « France
  Invoice UBL Extension ») + Télémétrie repliable. Aucune notion d'annuaire.
- Le **rapport HTML** (`report.rs::render`) est **100 % dérivé du `Snapshot` de télémétrie** —
  mêmes données que le cockpit. Il expose « Provisionnés **Réseau** Peppol » (`s.exists`, constat
  SML/SMP du run).
- Les 5 notions ne circulent aujourd'hui **que vers le CSV**, calculées à l'export par ligne
  dans `output.rs` (`in_directory` : `output.rs:280-287` ; `PpfFlags` : `output.rs:291-298`).
  Elles reposent sur `store::directory_present(&[String]) -> HashSet<String>` (`store.rs:293`)
  et `store::ppf_flags(&[String]) -> HashMap<String, PpfFlags>` (`store.rs:428`).
- **Il n'existe donc aucun agrégat de présence annuaire.** Il faut le créer.

## Décisions de cadrage (validées)

1. **Modèle temporel** : le panneau se remplit **dès le chargement** (entrée + annuaires),
   **avant le run** — donnée déclarative statique, indépendante de la résolution API. Pas de
   branchement dans le `Snapshot` de télémétrie (ce serait un faux signal : chiffres statiques
   animés au rythme du run).
2. **Dénominateur** : **éligibles 0225** (SIRENE FR), comptés **par ligne d'entrée** (cohérent
   avec le comptage par ligne du reste de l'app — `Snapshot.*_lines`). Les lignes non-0225 sont
   **exclues** du calcul et affichées **en clair** comme « non applicables ».
3. **Distinction sémantique** : « présence en **annuaire** » (fichier déclaratif) ≠ « Provisionnés
   **Réseau** Peppol » (constat réseau du run). Portée par le **titre du panneau**, une **légende
   explicite**, et une **forme différente** (barres, pas anneaux).

## Sémantique exacte du calcul

Pour chaque **ligne** d'entrée :

- Le PID est-il un identifiant **0225** ? Sinon → « non applicable » (compté à part, exclu du
  dénominateur). La valeur 0225 est extraite comme dans `directory.rs::parse_0225_value`
  (préfixe `iso6523-actorid-upis::0225:` retiré).
- Si 0225 → la ligne entre dans le **dénominateur** (`eligible_0225`). Sa valeur est cherchée :
  - dans l'ensemble `directory_present` → incrémente `peppol.present` si trouvée.
  - dans la map `ppf_flags` → incrémente `ppf.present / active / pdp_definie / usable` selon les
    drapeaux de l'entrée trouvée (une entrée absente de la map compte 0 partout, comme le
    `Some(défaut)` de `output.rs`).

Comptage **par ligne** : une même valeur 0225 sur deux lignes compte deux fois (aligné sur le
comptage par ligne existant). Les requêtes DB restent dédoublonnées par valeur (lots de 500,
comme aujourd'hui) ; seul le **rattachement aux lignes** rétablit le compte par ligne.

## Architecture

### Fonction pure de couverture (testable, TDD)

Nouveau module **`coverage.rs`** (module étanche, cf. convention projet) exposant :

```rust
pub struct PeppolCoverage { pub present: usize }

pub struct PpfCoverage {
    pub present: usize,      // annuaire_ppf
    pub active: usize,       // ppf_active
    pub pdp_definie: usize,  // pdp_definie
    pub usable: usize,       // ppf_usable
}

pub struct Coverage {
    pub total_lines: usize,       // lignes d'entrée
    pub eligible_0225: usize,     // dénominateur
    pub non_applicable: usize,    // lignes non-0225 (affiché en clair)
    pub peppol: Option<PeppolCoverage>, // None = annuaire Peppol non chargé (gate)
    pub ppf: Option<PpfCoverage>,       // None = annuaire PPF non chargé (gate)
}
```

Le **cœur pur** ne touche pas la DB. Signature indicative :

```rust
pub fn compute(
    eligible_values: &[String],            // valeurs 0225 par ligne (doublons conservés)
    non_applicable: usize,
    present: Option<&HashSet<String>>,      // None => bloc Peppol masqué
    ppf: Option<&HashMap<String, PpfFlags>>,// None => bloc PPF masqué
) -> Coverage
```

Les `Option` encodent le **gate** : `present`/`ppf` valent `None` quand l'annuaire correspondant
n'est pas chargé, ce qui met le bloc `Option` de `Coverage` à `None`. Gates **indépendants**
(miroir exact des gates existants de `generate_output`).

### Deux points d'appel, une fonction

- **Cockpit (avant run)** : nouvelle commande Tauri `directory_coverage() -> Coverage`. Elle lit
  les PID d'entrée chargés, extrait les valeurs 0225 par ligne, interroge `directory_present` et
  `ppf_flags` **seulement si l'annuaire correspondant est chargé** (gate via
  `peppol_directory_status()` / `ppf_summary().distinct_addr > 0`), puis appelle `coverage::compute`.
  Appelée par `cockpit.js` à **l'entrée de l'étape 3** et après **(re)chargement / reset** d'un
  annuaire (les blocs annuaire vivent dans l'onglet Fichiers).
- **Rapport** : la `Coverage` est **calculée et figée dans `LastRun`** à la fin du run (population
  = l'entrée du run), pour que le rapport décrive exactement le run qu'il documente. `ReportData`
  reçoit `coverage: Coverage` ; `report::render` ajoute une section.

> **À résoudre au plan** : la source exacte des PID d'entrée pour le calcul pré-run (comment
> `commands.rs` accède aux lignes d'entrée chargées hors run — vérifier `csv_io` / `commands` /
> `resolver`). C'est le seul point d'implémentation non tranché ; il ne change pas la conception.

## UI — Cockpit

- **Emplacement** : nouveau bloc **sous `#biz-band`, avant `<details id="telemetry">`** (validé).
- **Forme** : barres horizontales (fond `--track`, remplissage coloré), pas d'anneaux — signale la
  nature déclarative/statique, distincte des tuiles-anneaux du run.
- **En-tête** : icône + « Présence en annuaire » + ligne d'éligibilité en clair : « **900**
  éligibles 0225 / 1000 lignes · **100** non applicables ». Légende : « Présence déclarative dans
  les annuaires chargés — distincte du "Provisionnés Réseau Peppol". »
- **Annuaire Peppol** : barre **verte** (`--green`) — cohérent avec l'accent `dir` de l'onglet
  Format et la sémantique « présence ». `present / eligible_0225` + %.
- **Annuaire PPF** : entonnoir violet (convention `--ppf-l1..l4`, large → strict) :
  - `Annuaire PPF` (présent) → `--ppf-l1`, en tête de groupe.
  - `PPF actif` (motif C/P) → `--ppf-l2`, sous-ligne.
  - `PDP définie` (réelle) → `--ppf-l3`, sous-ligne.
  - `PPF utilisable` (actif + PDP réelle) → `--ppf-l4`, sous-ligne **mise en avant** (métrique-clé).
- **États vides (gates indépendants)** : annuaire non chargé → bloc correspondant masqué + indice
  discret invitant à déposer l'export dans l'onglet Fichiers. **Aucun** annuaire chargé → panneau
  entier masqué.
- Sécurité UI : construction DOM via `h()` / `textContent` (jamais d'`innerHTML` dynamique).

## UI — Rapport HTML

- Nouvelle section `<h2>Présence déclarative en annuaire</h2>` (souligné or via `h2::after`),
  dans une carte `--card`, **après** les tuiles KPI réseau, clairement distinguée de
  « Provisionnés Réseau Peppol ».
- Même grammaire visuelle que le cockpit (barres, mêmes couleurs, même entonnoir PPF).
- Légende rappelant le dénominateur (900 éligibles 0225) et la distinction annuaire ≠ réseau.
- **Ajout requis** : variables `--ppf-l1..l4` au CSS de `report.rs` (aujourd'hui absentes) **et**
  leur déclinaison **thème clair** (le rapport supporte `prefers-color-scheme` + `data-theme`).
- Section rendue **uniquement si** au moins un annuaire est présent dans la `Coverage`
  (`peppol.is_some() || ppf.is_some()`).

## Tests (TDD — test d'abord)

`coverage::compute` est le cœur métier, testé sans DB ni UI :

1. **Peppol seul chargé** : `ppf = None` → `Coverage.ppf == None`, `peppol` compté.
2. **PPF seul chargé** : symétrique.
3. **Aucun annuaire** : `peppol == None && ppf == None`.
4. **Dénominateur** : mélange 0225 / non-0225 → `eligible_0225` et `non_applicable` corrects ;
   les non-0225 n'entrent ni au numérateur ni au dénominateur.
5. **Comptage par ligne** : même valeur 0225 en double → comptée deux fois.
6. **Entonnoir PPF** : cas discriminant `usable != active && pdp_definie` — une valeur (C,1)+(V,0)
   compte en `active` et `pdp_definie` mais **pas** en `usable` (miroir du test `id_split`,
   `store.rs:872`).
7. **Absent de l'annuaire** : valeur 0225 éligible mais absente de `present` / `ppf_flags` →
   compte au dénominateur, 0 au numérateur (distinct de « non applicable »).
8. **Round-trip serde** de `Coverage` (contrat cockpit JS ↔ rapport).

Pas de parité CLI (le CLI `popaul.py` n'a ni cockpit ni rapport — comme les 4 champs PPF).

## Hors scope

- Toute jointure/croisement PPF ↔ réseau Peppol ↔ CTC (rester sur des compteurs indépendants).
- Rafraîchissement live pendant le run (explicitement écarté).
- Export CSV de la couverture agrégée (le CSV par ligne existe déjà).
- Historique / persistance de la couverture au-delà de `LastRun`.

## Risques / points d'attention

- **Collision verte** cockpit : « Annuaire Peppol » (vert) proche de « Provisionnés Réseau Peppol »
  (vert). Atténuée par forme (barres vs anneaux) + titre + légende. Repli possible en or si l'usage
  révèle une confusion (validé : on reste en vert).
- **Cohérence population** cockpit (live, entrée courante) vs rapport (figé sur l'entrée du run) :
  léger décalage possible si l'entrée change entre run et export ; acceptable — chaque vue est
  correcte pour sa population.
- Thème clair du rapport : ne pas oublier la déclinaison `--ppf-l*` (sinon variables non définies).
