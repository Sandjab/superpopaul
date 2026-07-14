# Modale de calibration — spec

**Date** : 2026-07-14 · **Sous-projet** : superpopaul · **Statut** : validé (maquette
`.superpowers/brainstorm/48654-1784023666/content/calibrate-modal.html`, 3 états)
· Évolution de `2026-07-14-calibrate-bench-design.md` (le banc quitte le panneau ⚙).

## Objet

Le banc d'essai ne vit plus dans le panneau ⚙ (dont il changeait la taille) mais
dans la **modale partagée** (`#modal`, z-index 50) ouverte au clic sur « Calibrer ».
Le cycle devient explicite : bouton **Arrêter** pendant la mesure, puis
**Retenter / Ignorer / Appliquer** à la fin. **L'application automatique de la
concurrence disparaît** — seul « Appliquer » écrit dans les réglages.

## Terminologie

« **Calibration** » dans tous les textes UI et messages d'erreur (« Calibration
en cours… », « Calibration terminée », « Calibration arrêtée », « Calibration
annulée. », « Calibration impossible : … »). Le bouton du panneau ⚙ reste
« **Calibrer** ». Les commentaires de code ne sont pas concernés.

## Les trois états de la modale

1. **En cours** — titre « Calibration en cours… », le banc se construit
   (événements `calibrate-step`, rendu inchangé), ligne d'état « palier N
   sessions — mesure… · X adressages consommés », un seul bouton :
   **■ Arrêter** (bordure/texte `--red`). Backdrop et Échap : inertes.
2. **Terminée** — titre « Calibration terminée », banc figé (dim des perdants),
   ligne d'état = verdict actuel (vert). Boutons : **↻ Retenter** (bordure/texte
   `--blue`), **Ignorer** (neutre), **✓ Appliquer N sessions** (fond vert plein,
   action principale — le libellé porte la valeur). Backdrop et Échap = Ignorer.
3. **Arrêtée (partiel)** — titre « Calibration arrêtée », ligne d'état « arrêtée
   au palier N · meilleur mesuré : X sessions, ~Y adr/s · Z adressages
   consommés ». Mêmes 3 boutons ; **Appliquer est désactivé si aucun palier
   complet** (aucun verdict reçu — un rapport `(1, 0.0)` ne doit pas être
   applicable).

Actions : **Appliquer** = écrit `best_concurrency` dans les champs Concurrence
(API + miroir direct) et `state.config`, ferme la modale. **Ignorer** = ferme
sans rien changer. **Retenter** = ferme et relance le flux complet (nouveau
calibrage, banc remis à zéro). Dans tous les cas le verdict texte reste comme
résumé dans `#calibrate-result` (suffixe « — appliquée » si appliquée).

## Backend (Rust)

- `calibrate()` prend un paramètre d'annulation (`&AtomicBool`) testé **en tête
  de chaque palier** : l'arrêt est coopératif, le palier en cours se termine
  (« fige à la fin du palier en cours »). `CalibrationReport` gagne
  `cancelled: bool`.
- `AppState` porte le flag (`Arc<AtomicBool>`), remis à `false` au début de
  chaque `calibrate_api`. Nouvelle commande `cancel_calibration` qui l'arme.
- Un palier gagnant mesuré avant l'annulation reste dans `best_concurrency` :
  le partiel est proposable (décision utilisateur du 2026-07-14).

## UI (vanilla JS)

- La modale réutilise `modal()`/`closeModal()` et le pattern
  d'ajout/retrait de listeners backdrop+Échap de `ensureProxyCreds`.
- Le div du banc est créé dynamiquement dans la modale avec `id="calibrate-bench"`
  (le CSS `.cal-*`/`#calibrate-bench` existant s'applique tel quel) ; le div
  statique du panneau ⚙ est supprimé de `index.html`.
- Erreurs de garde (prérequis, mode direct) : la modale ne s'ouvre pas —
  l'erreur s'affiche dans `#calibrate-result` comme aujourd'hui. La modale
  s'ouvre juste avant `invoke("calibrate_api")`, après `ensureProxyCreds`
  (séquencement compatible avec la modale proxy, singleton `#modal`).
- « Arrêter » : `invoke("cancel_calibration")`, bouton désactivé avec
  « arrêt en cours… » jusqu'à la résolution de l'invoke principal.
- Styles boutons : `.btn-stop` (bordure `--red`), `.btn-retry` (bordure
  `--blue`), `.btn-apply` (fond `#238636`, bordure `--green`, texte blanc,
  gras) + rangée `.modal-btns` alignée à droite.

## Tests

- Rust TDD : flag pré-armé → aucun pas émis, `cancelled: true` ; flag armé par
  la closure de progression au premier verdict → arrêt après ce palier,
  `cancelled: true`, `best_concurrency` = palier mesuré ; tests existants
  adaptés (`cancelled: false`). La commande `cancel_calibration` elle-même
  n'est pas unit-testée (injection Tauri, dérogation comme le relais).
- Chromium (stub `__TAURI__`) : nominal + Appliquer (champs écrits, modale
  fermée, résumé) ; Arrêter → état 3 partiel → Appliquer la valeur partielle ;
  Ignorer (champs INchangés — c'est le test de la fin de l'auto-application) ;
  garde → pas de modale.
