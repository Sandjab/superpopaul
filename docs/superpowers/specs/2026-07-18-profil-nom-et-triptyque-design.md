# Nom du profil chargé & triptyque Ouvrir/Enregistrer/Enregistrer sous — design

Validé le 2026-07-18 (maquette playground, variante B « icône + libellé court »,
état « nom • modifié »). Prolonge la refonte profils v1 mergée le même jour
(`2026-07-18-refonte-profils-onglets-design.md`).

## Objectif

Rendre visible le profil actif dans la barre de l'onglet Format, et remplacer
les deux boutons Charger/Sauvegarder par le triptyque standard :
📂 Ouvrir… / 💾 Enregistrer / Enregistrer sous…

## Périmètre

`client/src/index.html`, `client/src/app.js`, `client/src/styles.css`
uniquement. **Zéro Rust** : `save_profile`/`load_profile` prennent déjà un
chemin ; hash et validation restent côté Rust.

## État (app.js, session seulement — rien de persisté)

- `state.profile = null | { path, name }` — le profil courant ; `name` est le
  nom de fichier seul (basename), `path` le chemin complet du YAML.
- Un **instantané de référence** : sérialisation JSON de
  `{ pid_column, columns, encoding, separator }`, capturé au chargement d'un
  profil et à chaque enregistrement réussi (Enregistrer et Enregistrer sous…).
- **Modifié** = la même sérialisation calculée sur l'état courant diffère de
  l'instantané. Aucune instrumentation des points de mutation : le calcul se
  fait au rafraîchissement de l'affichage.

## Rafraîchissement

Une fonction `renderProfileBar()` (nom + « • modifié » + grisage des boutons)
appelée depuis les points de rendu d'app.js (entrée dans l'étape Format,
`designatePid`, listeners encodage/séparateur, handlers profil, `pickInput`)
**et** exposée en hook optionnel global `window.updateProfileBar?.()` appelé en
fin de `renderOutPreview()` (columns.js) — même motif que l'existant
`window.updateRunModeHint?.()` : columns.js ne connaît pas la logique, il
signale juste qu'un rendu a eu lieu (drag, double-clic).

## Barre `#format-head`

`Format du fichier principal`, puis à droite du titre le nom du profil en
texte assourdi suivi de « • modifié » en doré (`--gold`) quand l'état diverge,
puis les trois boutons :

| Bouton | Comportement | Grisé quand |
|---|---|---|
| 📂 Ouvrir… | flux actuel (dialogue, refus sec sur hash) + mémorise `{path, name}` + prend l'instantané | jamais |
| 💾 Enregistrer | `save_profile` sur `state.profile.path`, sans dialogue ; reprend l'instantané ; erreur → bannière | pas de profil courant OU pas modifié |
| Enregistrer sous… | flux actuel (dialogue `save`) ; le fichier choisi devient le profil courant + instantané ; `defaultPath` propose le nom courant s'il existe (en plus du dossier portable) | pas de désignation (règle actuelle) |

## Cycle de vie du contexte profil

- Chargement réussi → `{path, name}` posés, instantané pris, barre à l'état
  « propre ».
- Enregistrer / Enregistrer sous… réussis → instantané repris (état
  « propre ») ; Enregistrer sous… met à jour `{path, name}`.
- Nouveau dépôt de fichier (`pickInput`) : si `columns_hash` diffère de celui
  du fichier précédent, le contexte profil est **effacé** (`state.profile =
  null`, instantané oublié) ; signature identique → conservé tel quel.
- Refus sec (hash incompatible au chargement) : aucun changement, comme
  aujourd'hui.

## Styles

Nom en classe assourdie existante (`muted`), « • modifié » via `var(--gold)`
(rôle « activité » de l'identité Bleu nuit & or — pas l'orange, réservé aux
avertissements). Aucune couleur en dur.

## Textes (français)

- Tooltips : 📂 « Ouvrir un profil YAML — appliqué si ses colonnes
  correspondent au fichier ouvert. » ; 💾 « Enregistrer — écrase le profil
  courant. » ; « Enregistrer sous… » : « Nouveau fichier YAML. »
- Indicateur : « • modifié ».

## Hors périmètre

Pas de confirmation avant écrasement (💾 est le comportement standard), pas de
persistance du profil courant entre sessions, pas de raccourcis clavier.

## Tests

Aucune logique métier nouvelle (comparaison de chaînes et grisage) : pas de
test Rust à ajouter, vérification manuelle en fin de chantier (charger,
modifier, enregistrer, enregistrer sous, re-dépôt même/différente signature).
