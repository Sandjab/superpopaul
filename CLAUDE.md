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
  Tests : `node --test "tests/*.test.js"` depuis `client/` (stdlib Node, aucune
  dépendance). `client/tests/dom_shim.js` fournit un faux DOM qui exécute le
  **vrai** `src/app.js` — réservé au câblage de l'UI (un champ reconstruit qui
  perd sa valeur, un écouteur qui ne rebranche pas) ; tout ce qui touche au
  rendu réel se vérifie dans l'application.
- Sécurité UI : **jamais d'innerHTML avec des données dynamiques** (contenu
  CSV, messages d'erreur backend) — construire le DOM via le helper `h()`
  de `app.js` ou `textContent`. Un CSV est une entrée non fiable.
- Sécurité : les identifiants proxy ne sont JAMAIS écrits sur disque
  (test `config::proxy_creds_never_serialized` — ne pas le contourner).
- Texte UI et messages d'erreur en **français**.
- IHM : **maquette HTML validée par un go explicite avant tout code UI**
  (nouvel écran, retouche visuelle, libellé compris). La maquette reprend la
  palette réelle « Bleu nuit & or » de l'application.
- Accessibilité : **explicitement abandonnée** (décision du 14/07/2026).
  Ne pas proposer d'améliorations a11y (contrastes, ARIA, navigation clavier).
- TDD : test d'abord pour toute logique Rust. Commits fréquents,
  format `feat(superpopaul): …` / `fix(superpopaul): …`.
- Releases : chaque release reçoit des **notes rédigées pour un humain**
  (`gh release edit vX.Y.Z --notes-file …` après la CI) ; leur publication
  sans relecture est autorisée en permanence — publier, puis montrer le texte
  publié. Lancer une release et pousser restent demandés à chaque fois.
  L'utilisateur ne peut tester **Windows** que via les releases GitHub : une
  demande de release peut donc précéder la validation GUI, c'est normal.
  Mécanique tag → push → attente CI → notes : `scripts/release-ci.sh`.
- Base locale (macOS) :
  `~/Library/Application Support/cloud.gavini.superpopaul/superpopaul.db`.
  Pour l'inspecter : `sqlite3 "file:$DB?mode=ro"` (lecture seule — l'app peut
  être ouverte). Tables principales : `resolutions`, `ppf_directory`,
  `peppol_directory` ; les adressages y sont stockés sous forme **courte**
  (sans préfixe `iso6523-actorid-upis::0225:`).
