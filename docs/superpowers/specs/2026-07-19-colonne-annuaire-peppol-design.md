# Colonne « annuaire Peppol » (présence déclarative, in_directory) — design

Validé le 2026-07-19 (maquette accent vert + icône 📇 ; sémantique : égalité
exacte, true/false, vide hors-0225 ; libellé « annuaire Peppol » ; vide +
avertissement si non chargé). Concrétise le croisement **déclaré/provisionné**
laissé hors périmètre du chantier d'ingestion
(`2026-07-18-annuaire-peppol-directory-design.md`).

## Objectif

Ajouter une colonne calculée `in_directory` au tableau de sortie (onglet
Format) : pour chaque ligne, indique si l'adressage est **déclaré dans
l'annuaire Peppol** (table `peppol_directory`), signal **distinct** de « existe »
(provisionné dans le réseau). Un participant peut être déclaré sans être
provisionné, et inversement.

## Sémantique (validée)

Par ligne, à partir de l'adressage brut `raw_pid` :
- `v = parse_0225_value(canonical(raw_pid))` (fonction déjà écrite,
  `directory.rs`) ;
- **0225** (`v = Some`) : **égalité exacte** de `v` dans `peppol_directory` →
  `"true"` si présent, `"false"` sinon ;
- **non-0225** (`v = None`) : `""` (l'annuaire 0225 ne couvre pas ce scheme —
  ni affirmer ni infirmer) ;
- **annuaire non chargé** (table absente) : `""` pour toutes les lignes.

Calcul **indépendant de la résolution** : la valeur ne dépend pas de la présence
d'une `Resolution` (un déclaré non-provisionné doit ressortir `"true"`).

## Périmètre

- **Rust** : `config.rs` (variant enum), `store.rs` (requête présence),
  `output.rs` (signature + calcul), `commands.rs` (`generate_output`).
- **Frontend** : `client/src/columns.js` (champ, libellé, icône, accent),
  `client/src/styles.css` (accent vert), `client/src/app.js` (avertissement
  annuaire non chargé).
- **Aucun** changement CLI/serveur. `PeppolField` est client-only : pas de
  parité `popaul.py`.

## Rust

### `config.rs`
Nouveau variant `PeppolField::InDirectory` (enum `#[serde(rename_all =
"snake_case")]`) → sérialisé `in_directory`, **qui est aussi l'en-tête CSV**
(`output::field_name`). Ajout **rétro-compatible** : les profils sans ce champ
restent lisibles ; la présence du champ dans un profil se sérialise
`{source: peppol, field: in_directory}`.

### `store.rs`
`fn directory_present(&self, values: &[String]) -> Result<HashSet<String>,
String>` : renvoie le sous-ensemble de `values` réellement présents dans
`peppol_directory` (`SELECT value FROM peppol_directory WHERE value IN (…)`, par
lots de 500 comme `load_map`). **Appelée uniquement quand la table existe**
(le commande garde via `peppol_directory_status()`).

### `output.rs`
- `field_name(InDirectory) => "in_directory"`.
- `generate(...)` gagne un paramètre `directory: Option<&HashSet<String>>` :
  `None` = annuaire non chargé **ou** colonne non demandée → cellule vide ;
  `Some(set)` = ensemble des valeurs 0225 présentes.
- Par ligne : `cpid = canonical(raw_pid)` calculé **une fois** (réutilisé pour
  `resolutions.get(&cpid)` et le calcul annuaire). La cellule `InDirectory` est
  calculée **hors du gate `res`** :
  ```
  None            → ""
  Some(set) → match parse_0225_value(&cpid) {
                   Some(v) if set.contains(&v) => "true",
                   Some(_)                     => "false",
                   None                        => "",
                 }
  ```
  Les autres champs Peppol restent calculés depuis `res` comme aujourd'hui (le
  bras `InDirectory` du match interne `res` est `unreachable!()`, traité en
  amont).

### `commands.rs` — `generate_output`
Après `load_map`, ne calcule la présence que si la colonne est sélectionnée :
```
wants_dir = cfg.output.columns contient Peppol{InDirectory}
directory =
  si wants_dir et peppol_directory_status() == Some :
     Some(directory_present(pids.filter_map(parse_0225_value)))
  sinon : None
```
Passe `directory.as_ref()` à `output::generate`. La distinction « demandée mais
non chargée » (pour l'avertissement UI) est gérée **côté frontend**, pas ici :
le contrat `generate_output -> Result<String,String>` est inchangé.

## Frontend

### `columns.js`
- `PEPPOL_FIELDS` : ajouter `["in_directory", "annuaire Peppol"]`.
- `PEPPOL_SAMPLE` : `in_directory: "true"`.
- `colLabel(c)` : icône **📇** pour `in_directory`, **⚡** pour les autres champs
  Peppol (aujourd'hui « ⚡ » pour tous).
- `makeHeader` : classe additionnelle `dir` sur le `th` quand
  `field === "in_directory"` (accent vert) ; la puce (`col-zone`) reçoit de même
  la classe `dir`.

### `styles.css`
- `#out-preview th.dir { color: var(--green); box-shadow: inset 0 0 0 1px var(--green); }`
- `.chip.dir { color: var(--green); border-color: var(--green); }`
  (même langage que `th.peppol` / `.chip.peppol`, couleur `--green` au lieu de
  `--gold`). Corps de colonne : atténué comme les autres champs calculés (aucun
  style dédié).

### `app.js` — avertissement annuaire non chargé
À la génération (`generate_output`), **si** la sortie contient la colonne
`in_directory` **et** que `directory_status()` renvoie `null` (annuaire jamais
chargé) → bannière : « La colonne « annuaire Peppol » est vide : l'annuaire n'a
pas été chargé (onglet Fichiers). » Non bloquant (la génération se fait quand
même). Test côté UI, cohérent avec le fait que la colonne sort vide côté Rust.

## Sécurité

Aucune donnée non fiable via innerHTML (libellés statiques, valeurs backend via
`textContent`/`h()`). Aucune écriture de secret. Le CSV de sortie ne contient
que `true`/`false`/vide pour cette colonne.

## Hors périmètre

- Correspondance sur le SIREN (préfixe) : rejetée — égalité exacte seule.
- Chargement des schemes autres que 0225 dans l'annuaire.
- Colonne dérivée « adressage annuaire correspondant » ou détail de la
  correspondance : seule la présence booléenne.
- Blocage de la génération quand l'annuaire n'est pas chargé (on avertit, on ne
  bloque pas).

## Tests (TDD Rust d'abord)

`store::tests` :
- `directory_present_renvoie_le_sous_ensemble_present` : charge un annuaire
  {a, b, c}, `directory_present([a, x, c])` → {a, c} (pas x).
- `directory_present_chunks` : > 500 valeurs traversent plusieurs lots.
- `directory_present_annuaire_vide` : table présente mais vide → HashSet vide.

`output::tests` :
- `in_directory_true_false_selon_presence` : deux lignes 0225, l'une dans le set
  passé, l'autre non → `"true"` / `"false"`.
- `in_directory_vide_hors_0225` : une ligne 0009 → `""`.
- `in_directory_vide_si_directory_none` : `directory = None` → `""` même pour un
  0225.
- `in_directory_independant_de_la_resolution` : une ligne sans `Resolution`
  (absente de la map) mais présente dans le set → `"true"` (le calcul ne passe
  pas par `res`).
- En-tête CSV : `in_directory` présent dans la ligne 1 quand la colonne est
  mappée.

Vérification manuelle finale : charger un fichier + l'annuaire réel, ajouter la
colonne, générer, contrôler true/false/vide sur des cas connus ; puis générer
sans annuaire chargé → colonne vide + bannière.
