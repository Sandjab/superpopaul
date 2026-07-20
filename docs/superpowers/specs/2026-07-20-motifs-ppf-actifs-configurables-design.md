# Conception — Motifs PPF « actifs » configurables

- **Date** : 2026-07-20
- **Statut** : validé (design), en attente de relecture spec avant plan
- **Composant** : `client/` (Rust + frontend). Fonctionnalité **client-only**, aucune parité `cli/`.

## Contexte

Les champs d'enrichissement PPF `ppf_active` et `ppf_usable` reposent aujourd'hui
sur un ensemble de motifs de présence **codé en dur** : `motif IN ('C','P')`. Cet
ensemble est écrit deux fois dans `store.rs::ppf_flags` (l'agrégat `active` et
l'agrégat `usable`).

Besoin exprimé : rendre cet ensemble **configurable** via une chaîne de motifs
(par ex. `"CP"`, `"CPN"`), pour pouvoir ajouter d'autres motifs (ex. `N`) sans
recompiler. `ppf_usable` doit **hériter** du même ensemble (une même ligne au
motif actif ET `pdp_fictive = 0`).

`ppf_active` et `ppf_usable` sont des **agrégats SQL calculés à la volée** à
chaque export / affichage de rapport ou cockpit — rien n'est stocké ni mis en
cache. Changer la configuration recalcule donc tout au prochain calcul, **sans
migration de données**.

## Objectif

Un réglage **global** « motifs PPF actifs » (défaut `"CP"`, comportement
identique à l'actuel), qui alimente `ppf_active` et, par héritage, `ppf_usable`.
`pdp_definie` et `annuaire_ppf` (présence) ne sont **pas** concernés.

## Décisions de conception

### Portée : réglage global

Le réglage vit dans les réglages de l'application (`Settings`, persistés dans
`superpopaul.yaml`), **pas** dans le profil de sortie. Justification : c'est une
règle d'**interprétation de l'annuaire PPF**, pas une préférence de mise en forme
de la sortie. Une valeur unique s'applique à tous les exports, rapports et
cockpits.

### Modèle de données

Nouvelle struct partagée, sur le modèle d'`ApiConfig` (déjà partagée entre
`Config` runtime et `Settings` persisté) :

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PpfConfig {
    #[serde(default = "ppf_active_motifs_default")]
    pub active_motifs: String, // ex. "CP"
}

impl Default for PpfConfig { /* active_motifs = "CP" */ }
```

- `Config { …, ppf: PpfConfig }` (runtime, disponible dans `cfg` au moment du
  calcul dans `commands.rs`).
- `Settings { …, ppf: PpfConfig }` (persisté).
- Dans les **deux**, le champ `ppf` porte `#[serde(default)]` : les YAML
  existants (sans `ppf:`) restent lisibles malgré `deny_unknown_fields` et
  prennent le défaut `"CP"` — soit **le comportement actuel exact**. Aucune
  migration.

### Sémantique de la chaîne de motifs

Hypothèses explicites (cohérentes avec l'existant : les motifs de l'export PPF
sont des codes **mono-lettres** — `C`, `P`, `V`, `N`…) :

- Chaque **caractère** de la chaîne est un motif. `"CPN"` → l'ensemble
  `{C, P, N}`.
- **Insensible à la casse** : la saisie est normalisée en majuscules, et la
  comparaison SQL se fait sur `UPPER(motif)` (robuste même si l'annuaire
  contenait des motifs en minuscules).
- Les **espaces** sont ignorés ; les **doublons** dédupliqués (`"CPP "` →
  `{C, P}`).
- L'ordre n'a pas de sens (ensemble d'appartenance).

### Validation

Au point d'entrée des réglages (`Config::validate` et `Settings::validate`, via
un helper `validate_ppf`) :

- Après normalisation, l'ensemble doit être **non vide** (au moins un motif).
- Chaque caractère retenu doit être une **lettre ASCII** (`A`–`Z`). Tout autre
  caractère (chiffre, ponctuation) → erreur.
- Message d'erreur en français, explicite, ex. :
  « motifs PPF actifs : au moins une lettre (ex. CP) ».

La chaîne **brute** saisie par l'utilisateur est conservée telle quelle dans le
YAML (round-trip fidèle) ; la normalisation ne sert qu'à la validation et au
calcul.

### Calcul

`store.rs::ppf_flags` prend un paramètre supplémentaire : l'ensemble des motifs
actifs (déjà normalisés en majuscules) :

```rust
pub fn ppf_flags(&self, identifiants: &[String], active_motifs: &[String])
    -> Result<HashMap<String, PpfFlags>, String>
```

- Les deux occurrences de `motif IN ('C','P')` deviennent une clause paramétrée
  `UPPER(motif) IN (?, ?, …)` construite à partir de `active_motifs`.
- Le SQL compose désormais **deux** jeux de placeholders (motifs + identifiants) ;
  les paramètres sont assemblés dans l'ordre attendu.
- `pdp_definie` (`pdp_fictive = 0`) et `in_ppf` (présence) restent **inchangés**.
- `ppf_usable` reste `MAX(<motif actif> AND pdp_fictive = 0)` sur la **même
  ligne** : il hérite automatiquement du nouvel ensemble.

Les **trois** sites d'appel dans `commands.rs` (export, `coverage_from_scan`,
`securisation_from_scan`) transmettent les motifs issus de `cfg.ppf.active_motifs`
(parsés une fois). `coverage_from_scan` et `securisation_from_scan` reçoivent
l'ensemble parsé en paramètre.

### Interface utilisateur

Un champ texte **« Motifs PPF actifs »** (défaut `CP`) dans l'écran des réglages
de l'application, près des autres réglages globaux (chargés/enregistrés via
`load_settings` / `save_settings`).

Conformément à la convention du projet, une **maquette HTML est validée** avant
d'écrire le code de l'IHM (voir la mémoire « maquette avant code UI »).

### Documentation & libellés

- Les tooltips de `client/src/columns.js` (`PPF_TIP`) qui citent « motif C ou P »
  deviennent **génériques** : « motifs configurés (par défaut C / P) ». On ne
  câble **pas** la valeur courante dans le tooltip (éviterait de passer la config
  au frontend `columns.js` pour un gain marginal).
- `docs/legende_champs.md` : la description de `ppf_active` / `ppf_usable` est
  mise à jour pour indiquer que l'ensemble des motifs est **configurable**
  (défaut C / P). Le PDF `docs/legende_champs.pdf` est **régénéré** (charte SFR).

## Plan de tests (TDD)

Logique Rust d'abord (`cargo test` dans `client/src-tauri/`) :

1. `ppf_flags` avec un ensemble custom (ex. `["C","P","N"]`) : une ligne au motif
   `N` rend `active = true` (et `usable` si `pdp_fictive = 0`) ; le défaut `CP`
   ne le ferait pas — le test échoue si l'ensemble n'est pas honoré.
2. `pdp_definie` **inchangé** quel que soit l'ensemble de motifs.
3. `ppf_usable` = motif actif **ET** `pdp_fictive = 0` sur la **même** ligne, avec
   l'ensemble custom (deux lignes séparées ne suffisent pas).
4. Insensibilité à la casse : annuaire avec motif `c`, réglage `"C"` → `active`.
5. Validation : chaîne vide, chaîne avec caractère non-lettre → `Err` ; dédup et
   espaces tolérés.
6. Round-trip serde `Settings`/`Config` avec `ppf.active_motifs`.
7. Rétro-compat : un YAML `Settings` **sans** section `ppf:` se charge et donne
   `active_motifs = "CP"`.

## Hors périmètre

- Parité `cli/` (la fonctionnalité PPF est client-only).
- Motifs **multi-caractères** (l'export PPF n'en produit pas).
- Ensembles **différents** entre `ppf_active` et `ppf_usable` (héritage assumé).
- Migration de données (le calcul est à la volée, rien n'est stocké).
