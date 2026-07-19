# Sécurisation de la montée en charge + titres d'unité (rapport HTML)

Spec de conception. Date : 2026-07-19.

## Objectif

1. **Titres d'unité** sur deux sections existantes du rapport HTML (contenu **inchangé**) :
   - « Plateformes de dématérialisation constatées » → suffixe « · en adressages uniques ».
   - « Présence déclarative en annuaire » → suffixe « · en {record_plural} ».
2. **Nouvelle section « Sécurisation de la montée en charge »** (uniquement en lignes du fichier,
   libellées avec `record_label`) : proportion des lignes prêtes sur tous les axes de la
   facturation électronique FR, en deux sous-parties **Entonnoir d'attrition** + **Synthèse**.

## Décisions validées (maquette v2 approuvée)

- **Population** : les lignes du **fichier courant**, avec le **dernier état de résolution connu en
  base** (`store::load_map`). Une ligne jamais résolue → non provisionnée. (Calcul post-run, à
  l'export, comme la couverture.)
- **Dénominateur** : le **total des lignes** du fichier principal.
- **Unité / libellé** : partout où le sens est « une ligne du fichier », on affiche le pluriel
  `record_label` (« CFs », « clients »…), **jamais le mot « ligne » en dur** (le label peut valoir
  « ligne » au sens *ligne téléphonique*). Formulations à **sujet neutre invariant** (« la part »,
  « proportion ») pour éviter tout accord d'adjectif avec un label dynamique.
- **Drapeaux par ligne** (prédicats exacts, réutilisant le canon du CSV) :
  - `in_peppol` (réseau) = `Resolution.exists_in_peppol == Some(true)`.
  - `ctc_ready` (extension FR prête aujourd'hui) = `output::ctc_status(r, now) == "ready"`
    (c.-à-d. `extended_ctc_fr == Some(true)` ET `ctc::state == Ready`) — **réutilisé**, pas
    redupliqué (parité colonne CSV `ctc_status`).
  - `ppf_usable` = drapeau PPF `usable` de l'identifiant 0225.
  - `in_directory` = présence 0225 dans l'annuaire Peppol chargé.
- **Représentation** (une section, deux sous-titres) :
  - **Entonnoir d'attrition** (cumulatif, ordre fixe) : `provisionnés réseau Peppol` →
    `+ extension FR prête` → `+ PPF utilisable` (= **cœur sécurisé**) → `+ annuaire Peppol`
    (= **pleinement sécurisés**). Dégradé vert `--sec-1..4` (large → strict).
  - **Synthèse** : 2 grands chiffres (cœur sécurisé, pleinement sécurisés) + une ligne
    « composantes » (chaque axe pris seul : PPF utilisable, provisionnés, extension FR prête).

## Sémantique du calcul (par ligne, pondéré par `line_counts`)

Pour chaque ligne d'entrée (poids = nombre de lignes du PID canonique) :

| niveau (cumulatif) | condition |
|---|---|
| `provisionnes` | `in_peppol` |
| `avec_extension` | `in_peppol ∧ ctc_ready` |
| `coeur` | `in_peppol ∧ ctc_ready ∧ ppf_usable` |
| `pleinement` | `in_peppol ∧ ctc_ready ∧ ppf_usable ∧ in_directory` |

Composantes autonomes (non cumulatives) : `ppf_usable_seul`, `ctc_ready_seul` (et `provisionnes`
sert aussi de composante « provisionnés »). Non-0225 → `ppf_usable = in_directory = false`
(ne peut donc jamais entrer dans `coeur`/`pleinement`).

Chaque niveau est un **sous-ensemble strict** du précédent (chaîne emboîtée).

## Architecture

### Module pur `securisation.rs` (testable, TDD)

```rust
pub struct LineFlags {
    pub weight: usize,       // nombre de lignes du PID
    pub in_peppol: bool,
    pub ctc_ready: bool,
    pub ppf_usable: bool,
    pub in_directory: bool,
}

pub struct Securisation {
    pub total_lines: usize,
    pub provisionnes: usize,    // in_peppol
    pub avec_extension: usize,  // + ctc_ready
    pub coeur: usize,           // + ppf_usable
    pub pleinement: usize,      // + in_directory
    pub ppf_usable_seul: usize, // composante
    pub ctc_ready_seul: usize,  // composante
}

pub fn compute(lines: &[LineFlags]) -> Securisation
```

`compute` somme les poids par condition. Pur, sans DB ni UI. `Serialize` (contrat rapport ; pas
d'usage JS pour l'instant — section rapport seule).

### Impur `securisation_from_scan` (commands.rs)

À l'export, sur un scan déjà fait :
- **Gate** : renvoie `Ok(None)` si l'un des deux annuaires n'est pas chargé
  (`peppol_directory_status().is_none()` ou `ppf_summary().distinct_addr == 0`) — sinon
  `coeur`/`pleinement` seraient des zéros trompeurs.
- Sinon : `load_map` (résolutions) + `directory_present` + `ppf_flags`, construit un `LineFlags`
  par PID (drapeaux via les prédicats ci-dessus, `output::ctc_status` réutilisé), puis
  `securisation::compute`.

### Intégration rapport

- `ReportData` gagne `securisation: Option<&Securisation>` (None → section non rendue).
- `export_report` (déjà async) : **un seul** `scan_unique_pids`, d'où l'on calcule **et** la
  couverture **et** la sécurisation ; tolérant (entrée illisible → couverture `EMPTY`,
  sécurisation `None`). `now = Utc::now()` pour `ctc_status`.
- `report::render` appelle `securisation_section` si `securisation.is_some()`, placée **après** la
  section « Présence déclarative en annuaire ».
- Réutilise `output::ctc_status` (passé en `pub(crate)`).

## UI (rapport)

- Section `<h2>Sécurisation de la montée en charge · en {record_plural}</h2>` (souligné or), carte.
- Sous-titre neutre : « Sur {total} {record_plural} — la part prête sur tous les axes de la
  facturation électronique française. »
- **Entonnoir** : 4 barres (mêmes classes que la couverture : `.cov-row`/`.bar`), remplissage
  `--sec-1..4`, largeur = niveau / total.
- **Synthèse** : 2 tuiles (réutilise `.kpi`/`.kpis` du rapport) — cœur (%) et pleinement (%), avec
  l'absolu en `{record_plural}` ; + ligne « composantes ».
- **CSS ajouté** : `--sec-1..4` (racine + thème clair + impression), un style de sous-titre, la
  teinte `.kpi` verte de sécurisation. Aucune donnée dynamique en innerHTML (rendu Rust côté
  serveur, valeurs via `fmt_int`).

## Tests (TDD)

`securisation::compute` (pur) :
1. **Emboîtement** : une ligne tout-vrai compte à tous les niveaux ; retirer `in_directory` →
   `coeur` oui, `pleinement` non ; retirer `ppf_usable` → `avec_extension` oui, `coeur` non.
2. **Pondération** : poids > 1 compté N fois.
3. **Composantes** : `ppf_usable_seul`/`ctc_ready_seul` comptent indépendamment de `in_peppol`.
4. **Non-0225** (ppf_usable=false,in_directory=false mais in_peppol/ctc_ready=true) → entre dans
   `provisionnes`/`avec_extension`, jamais `coeur`/`pleinement`.

Rapport (`report.rs`) :
5. Section rendue avec `Some(securisation)` → titre, sous-titres « Entonnoir d'attrition » /
   « Synthèse », les 4 niveaux, les 2 chiffres, la ligne composantes ; libellé `record_plural`
   présent, **aucun** « lignes » codé en dur quand le label ≠ « ligne ».
6. Section absente si `securisation == None`.
7. Titres d'unité présents : « · en adressages uniques » (PA) et « · en {record_plural} »
   (couverture).
8. Parité `ctc_ready` : test dédié dans `securisation_from_scan` (ou intégration) montrant qu'une
   résolution `extended_ctc_fr=Some(true)` + activation future compte en `ctc_ready = false`
   (miroir de `output::ctc_status` → « later »).

Pas de parité CLI.

## Hors scope

- Le double compte (adressages + lignes) sur PA/couverture (abandonné — on n'ajoute que l'unité au
  titre).
- Toute modification des **calculs** existants (KPI/anneau/PA/couverture inchangés).
- Population cumulée cross-fichier (lignes hors du fichier courant).
- Exposition de la sécurisation au cockpit (rapport seul).

## Risques

- **Gate strict (2 annuaires)** : si un seul annuaire est chargé, la section disparaît
  entièrement (pas de rendu partiel). Assumé : un cœur sécurisé sans PPF chargé n'a pas de sens.
- **Coût export** : `load_map` + `directory_present` + `ppf_flags` + scan — même famille de
  requêtes que `generate_output`, exécuté une fois à l'export (spawn_blocking). Acceptable.
- **Population ≠ KPI** : cette section est « fichier courant, dernier état base », distincte du
  « ce run » des KPI — c'est voulu et porté par le libellé d'unité et le sous-titre.
