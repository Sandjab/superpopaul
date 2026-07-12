# Super Popaul — encodage & séparateur de sortie (design)

Date : 2026-07-12 · Statut : validé

## Objectif

Donner un vrai rôle aux réglages de format : l'utilisateur choisit l'encodage
et le séparateur du **fichier de sortie**. L'entrée reste 100 % sniffée
(aucun changement à l'étape 1). Les champs YAML `input.delimiter/encoding`,
jusqu'ici décoratifs (le sniff re-détecte toujours), disparaissent des
nouveaux YAML.

## Décisions actées

- **Encodages proposés** : `utf-8-bom` (défaut), `utf-8` (sans BOM),
  `windows-1252`. Pas de latin-1 (quasi-doublon de 1252), pas d'UTF-16.
  Le défaut `utf-8-bom` **est** le comportement actuel (Excel FR par
  double-clic) : aucun changement silencieux.
- **Séparateurs proposés** : `auto` (défaut = identique à l'entrée sniffée),
  `;`, `,`, `|`, tabulation. Le défaut `auto` est le comportement actuel.
- **Compat YAML** : `input.delimiter/encoding` tolérés en lecture
  (`#[serde(default, skip_serializing)]`), plus jamais écrits. Pas de bump
  de version ni de migration (app non distribuée, zéro YAML dans la nature).

## Schéma YAML

```yaml
output:
  path: ./clients_enrichis.csv
  timestamp_suffix: true
  encoding: utf-8-bom      # utf-8-bom | utf-8 | windows-1252
  separator: auto          # auto | ";" | "," | "|" | "\t"
  columns: [...]
```

- `OutputEncoding` et `OutputSeparator` sont des **enums Rust** sérialisés
  (convention `PeppolField`) : une valeur invalide est rejetée par serde au
  chargement, pas de validation manuelle.
- `#[serde(default)]` sur les deux : un YAML sans ces champs charge avec
  les défauts.
- La validation `input.delimiter.len() == 1` (`config.rs`) est retirée —
  champ inerte.

## Écriture (`output.rs`)

- **Séparateur** : `auto` → `meta.delimiter` sniffé ; sinon l'octet choisi.
- **Encodage** :
  - `utf-8-bom` : BOM `EF BB BF` + UTF-8 (chemin actuel) ;
  - `utf-8` : UTF-8 sans BOM ;
  - `windows-1252` : transcodage via `encoding_rs`, caractères non
    représentables remplacés par `?` (assumé, documenté dans l'infobulle).
- L'écriture atomique (`.tmp` + rename) est inchangée.

## UI — étape 3, fieldset « Fichier de sortie »

Deux `select` sous le chemin, chacun avec infobulle `title` :

| Select | Options (libellés) | Infobulle |
|---|---|---|
| Encodage | UTF-8 avec BOM (Excel FR) · UTF-8 sans BOM · Windows-1252 | Le défaut garantit les accents dans Excel FR. En Windows-1252, les caractères non représentables deviennent « ? ». |
| Séparateur | Identique à l'entrée · Point-virgule ; · Virgule , · Pipe \| · Tabulation | Séparateur du fichier de sortie. « Identique à l'entrée » reprend celui détecté à l'étape 1. |

`syncOutputForm`/`fillOutputForm` (`app.js`) gagnent les deux champs ;
défauts dans `state.config.output`.

## Tests (TDD)

- `config` : un YAML « ancien » avec `input.delimiter/encoding` charge
  encore ; un YAML sauvegardé ne contient plus ces champs ; défauts
  appliqués si champs absents ; valeur d'encodage invalide rejetée.
- `output` : pas de BOM en `utf-8` ; `é` → octet `0xE9` en `windows-1252` ;
  caractère non mappable → `?` ; séparateur forcé `,` sur entrée `;` ;
  `auto` reprend le séparateur d'entrée.

## Hors périmètre

- Rien ne change à l'étape 1 (entrée sniffée) ni au preview.
- Pas d'UTF-16 (ni lecture ni écriture), pas de latin-1.
