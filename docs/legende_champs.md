# Légende des champs d'enrichissement

Super Popaul ajoute au fichier de sortie des **colonnes d'enrichissement** :
des champs calculés, à côté des colonnes reprises de l'entrée. Ce document
décrit, pour chaque champ, sa **signification**, ses **valeurs possibles** et
ses **règles de renseignement** (quand la cellule reste vide), regroupés par
**source**.

L'en-tête de chaque colonne dans le CSV est le **nom technique** (`in_peppol`,
`pa_code`, …). Le **libellé** est le nom affiché dans l'écran de configuration
des colonnes. L'ordre et le choix des colonnes sont configurables à l'étape
« Colonnes » — ce document décrit la sémantique, pas l'ordre.

## Conventions communes

- **Booléens** : toujours `true` / `false` / *(vide)* — jamais `1` / `0`.
- **Cellule vide** = information non disponible pour cette ligne. Le motif
  dépend de la source (adressage non résolu, annuaire non chargé, ou champ
  inapplicable à l'adressage) et est précisé pour chaque groupe ci-dessous.
- **Adressage « 0225 »** : un identifiant de la forme
  `iso6523-actorid-upis::0225:<valeur>` désigne un SIREN/SIRET français. Les
  deux annuaires (Peppol et PPF) sont indexés sur cette `<valeur>` nue. Les
  autres schemes (`0002`, `0009`, …) ne reçoivent **jamais** de valeur
  d'annuaire (cellule vide pour `in_directory` et les 4 champs PPF).
- Chaque ligne du fichier de sortie correspond à **une ligne de l'entrée** :
  deux lignes portant le même adressage reçoivent le même enrichissement.

---

## 1. Résolution Peppol ⚡

**Source** : le run de résolution (API REST ou résolveur direct). Ces champs
sont lus dans la base cumulée des résolutions, indexée par adressage.

**Règle de renseignement du groupe** : ces champs ne sont renseignés que si
l'adressage a été **résolu** (présent en base). Un adressage jamais résolu →
**toutes ces cellules sont vides**. Un `true`/`false` reflète le dernier état
connu ; un champ peut rester vide même après résolution si le catalogue SMP
n'a pas livré l'information (par ex. `existe = true` mais point d'accès
illisible).

| Colonne | Libellé | Signification | Valeurs |
|---|---|---|---|
| `in_peppol` | existe | L'adressage est-il provisionné dans le réseau Peppol (le SMP répond pour cet identifiant). | `true` / `false` / *(vide)* |
| `pa_code` | code PA | Code du point d'accès (Access Point) qui dessert l'adressage. | chaîne (ex. `PA0042`) / *(vide)* |
| `pa_name` | nom PA | Nom du point d'accès. | chaîne (ex. `ACME PA`) / *(vide)* |
| `pa_country` | pays PA | Code pays du point d'accès. | code ISO (ex. `FR`) / *(vide)* |
| `ubl_extended` | CTC-FR | L'adressage déclare-t-il le support de l'extension française **France Invoice UBL Extension** (CTC-FR). | `true` / `false` / *(vide)* |

---

## 2. Fenêtre de validité CTC ⚡

**Source** : dérivée du SMP au moment de la résolution. Les **dates** sont
stockées brutes ; l'**état** n'est jamais figé — il est recalculé au moment de
l'export à partir des dates. Un adressage « activation dans le futur » bascule
donc seul en `ready` le jour venu, sans nouvelle résolution.

**Règle de renseignement du groupe** : renseigné seulement pour un adressage
**résolu** (sinon vide, comme le groupe 1).

| Colonne | Libellé | Signification | Valeurs |
|---|---|---|---|
| `ctc_activation` | activation CTC | Date d'activation déclarée du support CTC (chaîne SMP brute, ISO 8601). | ex. `2026-09-01`, `2026-09-01T00:00:00Z` / *(vide si absente)* |
| `ctc_expiration` | expiration CTC | Date d'expiration déclarée du support. | date ISO 8601 / *(vide si sans limite)* |
| `ctc_status` | état CTC | État du support **calculé à l'instant de l'export** à partir des dates ci-dessus. | `ready` / `later` / `expired` / *(vide)* |

**Détail de `ctc_status`** :

- `ready` — actif aujourd'hui : aucune borne, ou activation passée **et**
  expiration à venir.
- `later` — activation dans le futur : pas encore prêt, basculera seul en
  `ready` le jour de l'activation.
- `expired` — expiration dépassée : plus prêt.
- *(vide)* — l'adressage ne déclare pas l'extension CTC-FR (`ubl_extended`
  ≠ `true`) : il n'y a aucun état à calculer. (Également vide si l'adressage
  n'est pas résolu.)

---

## 3. Annuaire Peppol 📇

**Source** : jointure **déclarative** avec l'annuaire Peppol chargé (export des
participants). **Indépendant de la résolution** : un adressage déclaré mais
jamais résolu ressort quand même `true`/`false`.

| Colonne | Libellé | Signification | Valeurs |
|---|---|---|---|
| `in_directory` | annuaire Peppol | L'adressage 0225 figure-t-il dans l'annuaire Peppol chargé. | `true` / `false` / *(vide)* |

**Règle de renseignement** :

- `true` — l'adressage 0225 est présent dans l'annuaire chargé.
- `false` — l'adressage 0225 est absent de l'annuaire.
- *(vide)* — l'annuaire Peppol n'est pas chargé, **ou** l'adressage n'est pas
  un 0225 (seuls les SIREN français sont indexés).

---

## 4. Annuaire PPF 🏛️

**Source** : jointure avec l'export B2B du **Portail Public de Facturation**
(PPF). **Indépendant de la résolution et de l'annuaire Peppol.** Ne concerne
que les adressages **0225**.

Ces 4 champs forment un **entonnoir** : la présence est la base, l'usabilité le
critère le plus strict (`annuaire_ppf` ⊇ `ppf_active` / `pdp_definie` ⊇
`ppf_usable`).

| Colonne | Libellé | Signification | Valeurs |
|---|---|---|---|
| `annuaire_ppf` | annuaire PPF | Adressage présent dans l'annuaire PPF chargé (au moins une ligne). | `true` / `false` / *(vide)* |
| `ppf_active` | PPF actif | Au moins une ligne au **motif de présence `C` ou `P`**. | `true` / `false` / *(vide)* |
| `pdp_definie` | PDP définie | Au moins une ligne avec une **PDP réelle** (`pdp_fictive = 0`). | `true` / `false` / *(vide)* |
| `ppf_usable` | PPF utilisable | Au moins une **même** ligne au motif `C` ou `P` **ET** PDP réelle (`pdp_fictive = 0`). | `true` / `false` / *(vide)* |

**Règle de renseignement du groupe** :

- *(vide)* — l'annuaire PPF n'est pas chargé, **ou** l'adressage n'est pas un
  0225.
- `false` — l'adressage 0225 est **absent** de l'annuaire PPF (distinct de
  vide : la question a été posée, la réponse est négative).
- `true` / `false` selon les critères ci-dessus — l'adressage 0225 est présent.

**À propos des colonnes de l'export PPF** : chaque ligne de l'annuaire PPF
porte un `MOTIF_PRESENCE` (les motifs `C` et `P` sont ceux considérés comme
« actifs ») et un indicateur `UTILISE_PDP_FICTIVE` (`0` = PDP réelle,
`1` = PDP fictive). Un même identifiant peut apparaître sur **plusieurs
lignes** ; d'où la distinction entre « au moins une ligne » (`ppf_active`,
`pdp_definie`, chacun sur des lignes éventuellement différentes) et « une même
ligne » (`ppf_usable`, les deux conditions réunies sur la même ligne).
