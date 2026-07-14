# Banc d'essai du calibrage — spec

**Date** : 2026-07-14 · **Sous-projet** : superpopaul · **Statut** : validé (maquette
`.superpowers/brainstorm/42318-1784016576/content/bench-final.html`, option A retenue)

## Objet

Pendant un calibrage (`calibrate_api`), afficher dans le panneau ⚙ un mini-graphe
en barres qui se construit palier par palier, à la place de l'actuel texte statique
« calibrage en cours… ». Chaque barre = un palier de concurrence testé,
hauteur = débit mesuré (adr/s).

## Sémantique des couleurs

| Couleur | État du palier |
|---|---|
| bleu (`--blue`) | mesure en cours |
| gris (`--border`) | mesuré, dépassé par un palier suivant (estompé au verdict) |
| vert (`--green`) | retenu (meilleure concurrence) |
| rouge (`--red`) | rejeté : gain ≤ 15 % sur le meilleur — c'est lui qui arrête |
| jaune (`--amber`) | un 429 est survenu pendant sa mesure — arrêt rate-limit |

(Les hex de la maquette de brainstorming ont été remplacés à l'implémentation
par les variables du thème de `styles.css` — sémantique identique.)

Règles :
- Au plus une barre rouge OU jaune (le palier d'arrêt), jamais les deux.
- Jaune prime sur rouge : si un palier cumule 429 et gain insuffisant, il est jaune
  (mesure non fiable).
- Arrêt par plafond (`concurrency.max(16)` atteint) : aucune barre colorée d'arrêt,
  le verdict texte suffit.
- Valeur adr/s affichée au-dessus de chaque barre mesurée ; celle du vert en vert,
  celle du rouge/jaune assortie.
- Un palier d'arrêt à débit nul (ex. 429 dès la première requête) garde une barre
  visible : plancher de 4 px (`min-height` sur `.cal-bar`).

## Backend (Rust)

- `resolver::calibrate()` prend un paramètre de progression supplémentaire
  (closure `Fn(CalibrationStep)`), appelée **2× par palier** :
  1. début de mesure : `{ level, status: Measuring }`
  2. fin de mesure : `{ level, addr_per_s, status: Retained | Rejected | RateLimited }`

  Nota : `Retained` est provisoire — un palier retenu peut être dépassé par le
  suivant. À l'écran : le palier `Retained` passe vert et le vert précédent
  redevient gris. Le gagnant final est dans le `CalibrationReport` existant
  (inchangé) ; la fin du calibrage = résolution de l'`invoke("calibrate_api")`,
  pas un événement dédié.
- `commands::calibrate_api` relaie chaque pas en `app.emit("calibrate-step", …)` —
  même pattern que `telemetry` pour le run. `calibrate()` reste testable sans Tauri.
- Le verdict texte gagne la raison d'arrêt : `(16 : +4 % < 15 %, arrêt)` /
  `(16 : rate-limité, arrêt)`.

## Frontend (vanilla JS, aucune logique métier)

- Bloc `#calibrate-bench` caché sous la ligne Concurrence de `#api-fields`
  (index.html) ; DOM construit via `h()` — jamais d'innerHTML.
- Apparaît au clic sur Calibrer ; se remplit au fil des événements `calibrate-step` ;
  reste affiché avec le verdict tant que le panneau est ouvert ; réinitialisé au
  calibrage suivant.
- Hauteurs normalisées sur le meilleur débit vu (re-échelonnage quand un nouveau
  max arrive). Hauteur max ~52 px, apparition de la barre en transition CSS courte.
- La ligne `#calibrate-result` reste la source du verdict texte.
- Mode direct : sans objet (le calibrage y est refusé, comportement inchangé).

## Tests (TDD)

- Rust d'abord : la séquence de pas émise par `calibrate()` est capturée par la
  closure dans les tests wiremock existants (`tests_calibrate`) et vérifiée pour
  les 3 causes d'arrêt : gain insuffisant → dernier pas `Rejected` ; 429 → dernier
  pas `RateLimited` ; plafond → dernier pas `Retained`, pas de pas d'arrêt.
- Rendu JS : validation Chromium (stub `__TAURI__`, événements simulés), captures
  aux phases mesure et verdict.
