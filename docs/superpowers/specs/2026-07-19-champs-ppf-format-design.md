# Champs PPF dérivés (annuaire_ppf, ppf_active, pdp_definie, ppf_usable) — design

Validé le 2026-07-19 (maquette `scratchpad/maquette-ppf-champs.html` : icône
🏛️ pour la famille PPF ; couleur violette **en dégradé** encodant l'entonnoir
présence → utilisable ; sémantique en miroir de `in_directory`). Concrétise le
croisement **PPF ↔ résolutions** laissé hors périmètre du chantier d'ingestion
de l'annuaire PPF (`2026-07-19-ppf-directory-design.md`).

## Objectif

Ajouter **quatre** champs calculés au tableau de sortie (onglet Format), issus
de la jointure de chaque adressage avec la table `ppf_directory` (annuaire B2B
du Portail Public de Facturation). Ces champs qualifient la présence et
l'exploitabilité d'un destinataire côté PPF, signal **distinct** de Peppol
(`in_peppol`, `in_directory`).

Rappel de `ppf_directory` (schéma existant, `store.rs`) : `PRIMARY KEY
(identifiant, motif)` — un même `identifiant` peut avoir **plusieurs lignes**,
une par `motif` ; colonnes `motif TEXT`, `pdp_fictive INTEGER` ∈ {0,1}.

## Sémantique (validée)

Jointure par ligne, à partir de l'adressage brut `raw_pid` :
`v = parse_0225_value(canonical(raw_pid))` (fonction existante, `directory.rs`),
comparé à `ppf_directory.identifiant`.

Les quatre champs, pour un `v` **0225 présent** dans l'annuaire :

- **`annuaire_ppf`** — `true` : l'identifiant a **au moins une ligne** en table.
- **`ppf_active`** — `true` ssi **au moins une ligne** a `motif ∈ {C, P}`.
- **`pdp_definie`** — `true` ssi **au moins une ligne** a `pdp_fictive = 0`.
- **`ppf_usable`** — `true` ssi **au moins une même ligne** a `motif ∈ {C, P}`
  **ET** `pdp_fictive = 0`.

**Nuance essentielle** : `ppf_usable ≠ ppf_active ET pdp_definie`. Les deux
conditions doivent tenir sur la **même** ligne. Exemple séparateur — un
identifiant à deux lignes `(C, 1)` et `(V, 0)` : `ppf_active` = vrai,
`pdp_definie` = vrai, `ppf_usable` = **faux**.

Table de vérité des cellules de sortie (miroir exact de `in_directory`) :

| Cas | `annuaire_ppf` | `ppf_active` | `pdp_definie` | `ppf_usable` |
|---|---|---|---|---|
| Annuaire PPF **vide/absent** | `""` | `""` | `""` | `""` |
| pid **non-0225** (`v = None`) | `""` | `""` | `""` | `""` |
| pid 0225 **absent** de l'annuaire | `false` | `false` | `false` | `false` |
| pid 0225 **présent** | `true` | selon règle | selon règle | selon règle |

`motif` est comparé **tel que stocké** (majuscules `C`/`P`, format de l'export
B2B ; aucune normalisation de casse — les valeurs sont conservées verbatim par
`ppf::stream_ppf`).

Calcul **indépendant de la résolution** : la valeur ne dépend pas de la présence
d'une `Resolution` (un adressage déclaré au PPF mais non résolu doit ressortir
ses drapeaux) — donc traité hors du gate `res` dans `output`, comme `in_dir`.

## Périmètre

- **Rust** : `config.rs` (4 variantes enum), `store.rs` (struct `PpfFlags` +
  requête jointe), `output.rs` (signature + calcul), `commands.rs`
  (`generate_output`).
- **Frontend** : `client/src/columns.js` (champs, libellés, icône, accents),
  `client/src/styles.css` (dégradé violet).
- **Aucun** changement CLI/serveur. `PeppolField` est client-only : pas de
  parité `popaul.py` (comme `in_directory` et l'ingestion PPF).

## Rust

### `config.rs`
Quatre variantes ajoutées à `PeppolField` (`#[serde(rename_all = "snake_case")]`)
→ sérialisées `annuaire_ppf`, `ppf_active`, `pdp_definie`, `ppf_usable`, **qui
sont aussi les en-têtes CSV** (`output::field_name`) :

```rust
AnnuairePpf,
PpfActive,
PdpDefinie,
PpfUsable,
```

Ajout **rétro-compatible** : les profils sans ces champs restent lisibles ; leur
présence se sérialise `{source: peppol, field: <nom>}`.

### `store.rs`
Structure des drapeaux dérivés + requête jointe agrégée :

```rust
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PpfFlags {
    pub in_ppf: bool,      // annuaire_ppf — ≥1 ligne
    pub active: bool,      // ppf_active   — ≥1 ligne motif C|P
    pub pdp_definie: bool, // pdp_definie  — ≥1 ligne pdp_fictive=0
    pub usable: bool,      // ppf_usable   — ≥1 ligne (C|P) ET pdp_fictive=0
}

pub fn ppf_flags(&self, identifiants: &[String])
    -> Result<HashMap<String, PpfFlags>, String>
```

`SELECT identifiant, MAX(motif IN ('C','P')), MAX(pdp_fictive = 0),
MAX(motif IN ('C','P') AND pdp_fictive = 0) FROM ppf_directory WHERE identifiant
IN (…) GROUP BY identifiant`, par **lots de 500** (`params_from_iter`, motif
`directory_present`). Chaque `identifiant` renvoyé a `in_ppf = true` ; les
absents ne figurent pas dans la map (→ tout `false` côté appelant). **Appelée
uniquement quand la table est non vide** (garde côté commande).

### `output.rs`
- `field_name` : 4 bras (`AnnuairePpf => "annuaire_ppf"`, etc.).
- `write_output`/`generate` gagnent un paramètre
  `ppf: Option<&HashMap<String, PpfFlags>>` : `None` = annuaire PPF vide **ou**
  aucune colonne PPF demandée → cellules vides.
- Par ligne, **hors du gate `res`** (comme `in_dir`) : `v =
  parse_0225_value(&cpid)` ; pour chaque champ PPF, `""` si `ppf = None` ou
  `v = None`, sinon lookup `map.get(v)` → `fmt_bool` du drapeau (`None` en map =
  absent = tous `false`). Bras dédiés dans le `match` de construction de la
  ligne (les 4 variantes traitées avant le bras `res`, `unreachable!` dans le
  bras `res` comme `InDirectory`).

### `commands.rs` — `generate_output`
Gate calqué sur `directory` :
```
wants_ppf = colonnes.any(|c| c est une des 4 PeppolField PPF)
ppf = if wants_ppf {
    let s = store.lock();
    if s.ppf_summary()?.distinct_addr > 0 {
        let ids = pids.filter_map(parse_0225_value);
        Some(s.ppf_flags(&ids)?)
    } else { None }
} else { None }
```
Passé en `ppf.as_ref()` à `output::generate`.

## Frontend

### `client/src/columns.js`
- `PEPPOL_FIELDS` : +4 entrées, libellés
  `["annuaire_ppf", "annuaire PPF"]`, `["ppf_active", "PPF actif"]`,
  `["pdp_definie", "PDP définie"]`, `["ppf_usable", "PPF utilisable"]`.
- `PEPPOL_SAMPLE` : valeurs d'exemple (`annuaire_ppf:"true"`, `ppf_active:"true"`,
  `pdp_definie:"false"`, `ppf_usable:"false"`).
- `colLabel` : icône **🏛️** pour les 4 champs (sinon ⚡, `in_directory` → 📇).
- `colClass` : **4 classes distinctes** pour le dégradé —
  `ppf-annuaire`, `ppf-active`, `ppf-pdp`, `ppf-usable`.
- `makeHeader` : tooltip par champ (texte des règles ci-dessus, français).

### `client/src/styles.css`
Quatre variables (intensités croissantes, entonnoir large → strict) et les
règles d'accent en-tête + chip :

```css
--ppf-l1:#6f6aa8; --ppf-l2:#8a80d4; --ppf-l3:#a892ff; --ppf-l4:#c3b6ff;
#out-preview th.ppf-annuaire { color: var(--ppf-l1); box-shadow: inset 0 0 0 1px var(--ppf-l1); }
/* …ppf-active → l2, ppf-pdp → l3, ppf-usable → l4 ; */
.chip.ppf-annuaire { color: var(--ppf-l1); border-color: var(--ppf-l1); }
/* …idem pour les 3 autres. */
```

**Écart assumé à la convention « 1 couleur = 1 famille »** (CLAUDE.md / Rule 11) :
le dégradé impose 4 classes au lieu d'une — choix produit explicite (la
sémantique d'entonnoir prime ici sur l'uniformité de famille).

## Tests (TDD — test d'abord pour toute logique Rust)

- **`store::ppf_flags`** :
  - présence simple (une ligne) → `in_ppf` seul.
  - `active`/`pdp_definie`/`usable` isolés et combinés.
  - **cas séparateur** : deux lignes `(C,1)` + `(V,0)` → `active` **et**
    `pdp_definie` vrais, `usable` **faux** (encode le « ET même ligne »).
  - identifiant absent → pas dans la map.
  - traversée **multi-lots** (> 500 identifiants).
- **`output::generate`** : colonnes PPF → `true`/`false`/`""` ; pid non-0225 →
  `""` ; `ppf = None` → `""` sur les 4.
- **`config`** : round-trip serde des 4 champs (`field_name` ↔ variante), et
  lecture d'un profil les contenant.
- **Aucun** test CLI (PPF client-only).

## Hors périmètre

- Rapport HTML (`report.rs`) : aucune tuile/KPI PPF (les 4 champs ne sont que
  des colonnes de sortie).
- Croisement PPF ↔ CTC / verdict temporel.
- Normalisation de casse du `motif`.
