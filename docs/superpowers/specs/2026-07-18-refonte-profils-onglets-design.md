# Refonte des profils et des onglets 1–2 — design

Validé le 2026-07-18 (brainstorm + maquettes interactives playground, choix
entérinés écran par écran).

## Contexte et objectifs

Un profil décrit **comment parser l'entrée et générer la sortie** — plus
jamais *quel* fichier traiter. En conséquence :

- le profil perd le chemin du fichier d'entrée ;
- il gagne l'encodage et le séparateur de sortie (qui quittent les réglages ⚙) ;
- il gagne un hash des en-têtes d'entrée, qui vérifie la compatibilité d'un
  profil chargé manuellement avec le fichier ouvert ;
- l'onglet 1 devient un hub de dépôt de fichiers ; l'onglet 2 concentre toute
  la définition du format entrée/sortie du fichier principal.

## Hors périmètre (explicitement)

- **Aucune migration** : anciens profils et anciens `superpopaul.yaml` sont
  rejetés avec une erreur claire ; l'utilisateur recrée ses fichiers.
- **Aucune association automatique** profil ↔ fichier (pas de registre de
  profils). Le hash ne sert qu'à la vérification au chargement manuel.
- **Aucun code companion** : la disposition de l'onglet 1 laisse la place à de
  futures zones de dépôt nommées, rien n'est construit.
- La résolution des colonnes (`read_column`, mapping de sortie) reste
  **sensible à la casse** — le hash l'est aussi, donc aucune incohérence.

## Modèle de données (Rust, `config.rs`)

Nouveau format de profil, `version: 1` (rupture assumée) :

```yaml
version: 1
input:
  pid_column: SIREN
  columns_hash: "9f3a4b2c1d8e7f60"   # FNV-1a 64 bits, hex minuscule
output:
  encoding: utf-8-bom                 # type OutputEncoding existant
  separator: auto                     # type OutputSeparator existant
columns:
  - { source: input, name: SIREN }
  - { source: peppol, field: in_peppol }
```

- `ProfileInput` : `pid_column`, `columns_hash` (le champ `path` disparaît).
- `Profile` gagne `output` (encodage + séparateur, types existants réutilisés).
- `Profile::validate` :
  - `pid_column` non vide ;
  - `columns_hash` non vide ;
  - `columns` contient `Input { name == pid_column }` — invariant
    « la colonne d'adressage est obligatoire en sortie », qui subsume
    l'ancien « au moins une colonne ». Choix produit : une sortie sans la clé
    est injoignable (la reprise, elle, lit la base locale et n'en dépend pas).
- `profile_from_yaml` : parse strict (`deny_unknown_fields`) + validate.
  Le fallback ancien-format et le booléen `legacy` disparaissent.
- `OutputSettings` (réglages ⚙) : perd `encoding` et `separator`, garde
  `dir`, `suffix`, `timestamp_suffix`.

## Hash des en-têtes (`csv_io`)

- FNV-1a 64 bits, écrit maison (valeur persistée : le hash doit être stable
  entre versions de Rust, `DefaultHasher` est exclu ; pas de dépendance crypto
  pour un simple contrôle de compatibilité).
- Calculé sur les en-têtes **décodés** (Unicode) — donc invariant à l'encodage
  et au séparateur du fichier d'entrée, ce qui est le but recherché.
- Chaque en-tête est absorbé sous forme d'octets UTF-8, préfixés par leur
  longueur (pas d'ambiguïté `["ab","c"]` vs `["a","bc"]`).
- Ordre significatif, casse significative, pas de trim.
- Rendu en hexadécimal minuscule sur 16 caractères.
- Exposé à l'UI : champ `columns_hash` ajouté au payload de `preview_csv`.

## Commandes Tauri (`commands.rs`, `lib.rs`)

- Supprimés : `AppState.base`, `input_path()`, la commande
  `resolved_input_path` (+ enregistrement `lib.rs`), la structure
  `ProfileLoad` (retour direct du `Profile`). `config::resolve_relative` est
  supprimé s'il n'a plus d'usage (à vérifier au moment du retrait).
- `load_profile` / `save_profile` ne touchent plus au répertoire de base.

## Frontend

### Stepper

`1. Fichiers / 2. Format / 3. Run`. Gating : Format actif dès qu'un fichier
est chargé ; Run actif dès qu'une colonne d'adressage est désignée
(l'invariant garantit alors ≥ 1 colonne de sortie).

### Onglet Fichiers (hub)

Dropzone (redéposer = remplacer) + méta (nom, taille, séparateur et encodage
détectés) + mini-aperçu de 3 lignes en **lecture seule**. Le sélecteur de
colonne d'adressage quitte cet onglet.

### Onglet Format

De haut en bas (« entrée en haut, sortie en bas », lecture en flux) :

1. Titre + boutons `Charger profil… / Sauvegarder…` en coin de panneau
   (sortis du header global) ;
2. Ligne désignation : `Colonne des adressages : [dropdown]` ;
3. Tableau d'aperçu manipulable (paradigme drag existant conservé) ;
4. Zone des colonnes écartées (chips) ;
5. Ligne sortie : `Encodage [—] Séparateur [—]` (sortis de ⚙).

### Désignation de la colonne d'adressage

- **Hybride synchronisé** : la dropdown ET une clé 🔑 cliquable qui apparaît
  au survol des en-têtes de colonnes d'entrée. Deux gestes, un seul état
  (`state.config.input.pid_column`).
- **Pré-désignation d'office** par `suggest_pid_column` au chargement du
  fichier ; sans suggestion, dropdown sur « — choisir — » et Run verrouillé.
- La colonne désignée reste une colonne `Input` ordinaire : l'accent est
  **dérivé** au rendu en comparant le nom à `pid_column` (pas de troisième
  variante de `ColumnSpec`, pas d'état invalide possible).
- Non écartable : garde `pull` par élément dans Sortable + garde sur le
  double-clic ; jamais de chip pour la colonne désignée. La garde
  « minimum 1 colonne » devient redondante et disparaît.
- Désigner une colonne écartée la réintègre d'office ; l'ancienne colonne
  désignée redevient écartable.

### Typologie visuelle des colonnes

| Type | Accent | Icône |
|---|---|---|
| Entrée ordinaire | neutre (existant) | — |
| Adressage (désignée) | écru lumineux `#e8e4d8` | 🔑 |
| Peppol | or `#d9a83f` (existant) | ⚡ |
| Jointure companion (futur) | à définir | à définir |

Implémentation via les tokens de `styles.css`, aucune couleur en dur
(identité « Bleu nuit & or » ; l'écru distingue par la luminosité, pas par
une teinte nouvelle).

### Profils côté UI

- **Sauvegarder** : sérialise l'état de l'onglet Format — `pid_column`,
  `columns_hash` de l'aperçu courant, encodage, séparateur, `columns`.
  Nécessite un fichier chargé et une désignation faite (structurellement
  garanti : l'onglet est inaccessible sans fichier, la sauvegarde est grisée
  sans désignation).
- **Charger** : compare le `columns_hash` du profil à celui de l'aperçu.
  Incompatible → **refus sec** : bannière d'erreur simple (« Profil
  incompatible avec le fichier ouvert — colonnes différentes »), aucun état
  modifié. Compatible → application intégrale (désignation, columns,
  encodage, séparateur). Plus de saut automatique vers Run avec un fichier
  embarqué (le profil n'en a plus).

## Erreurs et cas limites

- Profil illisible / champs inconnus / hash incompatible → bannière d'erreur,
  état intact.
- Ancien `superpopaul.yaml` → erreur explicite au démarrage (comportement
  `load_settings_file` existant : montrer, pas avaler).
- Aucune suggestion de colonne → désignation manuelle exigée avant Run.

## Tests

- `csv_io` : hash — sensibilité à l'ordre, à la casse, non-ambiguïté du
  préfixage, valeur en dur (stabilité inter-versions).
- `config` : aller-retour YAML profil v1, rejet champ inconnu, invariant
  « pid dans columns », rejet des anciens formats (profil et réglages),
  réglages sans encodage/séparateur.
- Aucune logique métier dans l'UI, donc pas de tests front (convention
  projet) ; tout ce qui est testable vit en Rust (`cargo test`).

## Ordre d'implémentation

1. **Rust en TDD** : hash `csv_io` → `Profile` v1 + purge migrations
   (`config.rs`) → nettoyage `commands.rs`/`lib.rs`. Vérifiable par
   `cargo test` sans UI.
2. **UI** (`index.html`, `app.js`, `columns.js`, `styles.css`) : maquette déjà
   validée écran par écran dans le playground (désignation hybride,
   disposition entrée/sortie, hub avec mini-aperçu, accent écru).
