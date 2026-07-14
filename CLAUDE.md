# Super Popaul — conventions du projet

- Projet **indépendant**. Trois composants séparés : `client/` (app graphique Tauri), `server/`
  (API REST Python), `cli/` (clients batch `popaul.py` / `popaul.ps1`).
- **Parité de canonicalisation** : `client/src-tauri/src/pid.rs::canonical`
  et `cli/popaul.py::canonical` doivent rester identiques — tests miroir
  `pid::tests` ↔ `cli/tests/test_popaul.py`, toute évolution d'un côté est
  reportée de l'autre.
- Python : serveur sans framework (`http.server` threadé), dépendances
  limitées à `server/requirements.txt` ; `cli/popaul.py` stdlib pure.
  Tests : `python3 -m unittest discover -s tests` depuis `server/` ou `cli/`.
- Rust : modules étanches (`pid`, `config`, `store`, `modes`, `csv_io`, `api`,
  `telemetry`, `resolver`, `output`, `commands`). Toute logique métier est
  testable sans UI (`cargo test` dans `client/src-tauri/`).
- Frontend : vanilla HTML/CSS/JS, **pas de bundler ni de framework**.
  L'UI n'a aucune logique métier : elle invoque des commandes et affiche
  des événements.
  Dérogation unique : SortableJS 1.15.6 vendorisé (`src/vendor/Sortable.min.js`,
  MIT, fichier seul) pour le drag des colonnes de l'étape 2 — le DnD HTML5 est
  avalé par le handler drag-drop Tauri (requis pour le drop de fichier), et le
  mode `forceFallback` de Sortable donne un drag pointeur animé qu'un
  équivalent maison ne justifiait pas de réécrire.
- Sécurité UI : **jamais d'innerHTML avec des données dynamiques** (contenu
  CSV, messages d'erreur backend) — construire le DOM via le helper `h()`
  de `app.js` ou `textContent`. Un CSV est une entrée non fiable.
- Sécurité : les identifiants proxy ne sont JAMAIS écrits sur disque
  (test `config::proxy_creds_never_serialized` — ne pas le contourner).
- Texte UI et messages d'erreur en **français**.
- TDD : test d'abord pour toute logique Rust. Commits fréquents,
  format `feat(superpopaul): …` / `fix(superpopaul): …`.
