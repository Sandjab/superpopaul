# Super Popaul 🍿 — Design

*Spec validée le 2026-07-12. Application graphique standalone de résolution Peppol batch, version graphique de `popaul.py`.*

## Objectif et contraintes

Application desktop **Windows + macOS**, portable et non signée (notice d'ouverture fournie),
destinée à des **collègues non techniques**. Elle lit un CSV contenant une colonne
d'adressages électroniques Peppol, résout chaque adressage unique via l'API REST existante
(`/resolve` et `/resolve/batch`, auth par clé), et produit un CSV enrichi.

- Taille : **≤ 20 Mo** (au-delà : arbitrage explicite requis).
- Volumétrie : ~**500 k adressages uniques** par fichier ; le débit dépend de la clé
  (rate-limitée ou non) et de la concurrence — ne jamais le présumer.
- Pause/reprise **critique**, y compris entre sessions.
- Esthétique soignée, effet « Whaou », retours graphiques temps réel.

## Stack retenue

**Tauri 2** : backend Rust (tokio, rusqlite, serde_yaml, csv), UI HTML/CSS/JS vanilla dans la
webview système (WKWebView sur macOS, WebView2 sur Windows — préinstallé sur Win 10/11 récents,
sinon installeur Evergreen). Binaire attendu ~5-10 Mo.

Alternatives écartées : Wails/Go (binaires plus gros, cgo pénible, écosystème moins actif),
Python+pywebview (20-40 Mo, démarrage lent, faux positifs antivirus sur PyInstaller non signé).

## Architecture

Sous-répertoire autonome `superpopaul/` (avec ses propres `README.md` et `CLAUDE.md`) :
structure Tauri classique, `src-tauri/` (Rust) + `src/` (UI).

Quatre modules Rust étanches :

| Module | Responsabilité | Ne connaît pas |
|---|---|---|
| `csv_io` | Lecture streaming du CSV d'entrée (détection encodage/séparateur, entêtes + échantillon), écriture de la sortie selon le mapping | API, base |
| `store` | Base SQLite globale des résolutions | CSV, API |
| `resolver` | Moteur : dédoublonnage, calcul de la liste selon le mode, workers tokio, retry/backoff, états pause/reprise/arrêt, télémétrie | Format CSV |
| `config` | YAML : mapping CSV, paramètres API, seuil refresh | — |

L'UI dialogue via commandes Tauri (`invoke`) et reçoit la télémétrie par événements Tauri,
émis par lots (~4/s). Toute la logique vit en Rust ; le front ne fait que de l'affichage.

**Séparation base / sortie** : la base est la source de vérité des résolutions ; le CSV de
sortie est une projection — autant de lignes que l'entrée, les infos d'un adressage (qui peut
apparaître sur plusieurs lignes) venant toujours de la base. Génération en fin de run ou à la
demande, par jointure ligne-à-ligne entrée × base (colonnes Peppol vides si non résolu).

## Modèle de données

### Base SQLite

Globale et partagée entre toutes les configs (dossier données utilisateur :
`~/Library/Application Support/SuperPopaul/` ou `%APPDATA%\SuperPopaul\`) :

```sql
CREATE TABLE resolutions (
  participant       TEXT PRIMARY KEY,   -- canonique "iso6523-actorid-upis::0009:..."
  exists_in_peppol  INTEGER,            -- 1/0, NULL si échec
  pa_code           TEXT,
  pa_name           TEXT,
  pa_country        TEXT,
  extended_ctc_fr   INTEGER,            -- 1/0
  api_status        TEXT,               -- 'ok' | 'error:<code HTTP>' | 'error:network'
  resolved_at       INTEGER             -- epoch secondes UTC
);
```

Les échecs sont stockés : « jamais tenté » ≠ « tenté en échec » (le mode reprise propose de
re-tenter les échecs).

### Config YAML

Un seul fichier, sauvegardable/rechargeable, chemins **relatifs au YAML** (trio
config+CSV déplaçable de poste à poste) :

```yaml
version: 1
api:
  url: https://peppol.gavini.cloud
  key: "..."            # en clair — avertissement UI à la sauvegarde
  batch_size: 50        # 1..50 (1 = mode unitaire) — mode hybride
  concurrency: 8
  proxy:
    url: http://proxy:3128   # URL SEULE ; login/mot de passe JAMAIS persistés
  refresh_days: 30
input:
  path: ./clients.csv
  delimiter: ";"
  encoding: utf-8
  pid_column: siren
output:
  path: ./clients_enrichis.csv
  timestamp_suffix: true
  columns:              # ordre = ordre de sortie ; absence = exclusion
    - {source: input, name: siren}
    - {source: input, name: raison_sociale}
    - {source: peppol, field: exists}
    - {source: peppol, field: pa_code}
    - {source: peppol, field: pa_country}
    - {source: peppol, field: extended_ctc_fr}
```

**Sécurité** : la clé API est persistée (accès en lecture seulement, toléré, avertissement à
la sauvegarde). Les identifiants proxy sont saisis dans l'UI à l'ouverture de session,
**conservés en mémoire uniquement** ; un 407 en cours de run suspend le traitement et rouvre
la saisie.

## Modes de résolution

Définis sur les **adressages uniques** du fichier d'entrée :

- **full** : tout résoudre (re-résolution même si présent en base) ;
- **reprise** : résoudre les absents de la base + option « re-tenter les échecs » ;
- **refresh** : résoudre les absents + ceux dont `resolved_at` est plus ancien que
  `refresh_days` jours.

## Moteur, erreurs, pause/reprise

Chaque réponse API met à jour la base immédiatement (transaction par paquet) : un arrêt
brutal ne perd rien.

- **Pause** : les workers finissent leur paquet puis s'endorment ; reprise instantanée.
- **Arrêt** : idem puis fin de run ; sortie générable avec les données déjà en base.
- **Reprise inter-sessions** : au chargement d'un YAML, comparaison entrée × base ; si run
  incomplet détecté → popup « 342 118/500 000 résolus. Reprendre ? ».

Gestion des erreurs :

| Signal | Comportement |
|---|---|
| 401/403 | Suspension immédiate, bannière « Clé API invalide », ressaisie + test, reprise sur place |
| 407 | Même mécanique pour les identifiants proxy |
| 429 | `Retry-After` respecté ; si taux de 429 > seuil glissant : **backoff adaptatif AIMD** (concurrence ÷2 puis +1 progressif) avec info visible |
| 5xx / timeouts en rafale | **Circuit breaker** : suspension + re-test auto à intervalle croissant (30/60/120 s) + bouton « réessayer maintenant » |
| Erreurs réseau isolées | Retry/backoff silencieux par paquet, comptabilisées au dashboard |

**Calibrage** : bouton envoyant des salves de test à concurrence croissante ; suggère la
concurrence optimale pour la clé et alimente l'**ETA** (affichée avant lancement, recalée en
continu pendant le run).

## UI (choix maquettés et validés)

**Structure : wizard linéaire** + écran de run. Splash screen (~1 s, fenêtre dédiée) pendant
l'ouverture base/config. Charger un YAML saute directement à l'étape Run.

1. **Fichier d'entrée** — drop zone + parcourir ; détection auto séparateur/encodage ;
   entêtes + 5 lignes d'exemple ; sélection de la colonne d'adressage avec **suggestion
   automatique** (détection de motifs PID/SIREN) ;
2. **Colonnes de sortie** — **aperçu direct manipulable** : le tableau final avec vraies
   données d'exemple ; en-têtes déplaçables (drag latéral), masquables (✕), bouton
   « + Ajouter champ Peppol » (exists, pa_code, pa_country, extended_ctc_fr) ;
3. **Sortie & API** — chemin de sortie (suffixe timestamp proposé), URL API + clé avec
   boutons « Tester » unitaires, proxy (URL seule persistée), concurrence, taille de paquet,
   bouton « Calibrer » ;
4. **Run — cockpit sombre** : ring héros (%, absolus dessous, ETA), tuiles % présents Peppol,
   % CTC-FR, **débits req/s ET adressages/s**, concurrence courante ; graphes répartition
   codes HTTP et latences (min/moy/p50/p90/p99/max) ; Pause/Stop ; bannières d'erreurs
   intelligentes.

À tout moment : « Sauvegarder la config ». Thème sombre.

## Tests

Logique critique testée en Rust sans UI :

- dédoublonnage + canonicalisation des PIDs (parité avec `popaul.py`) ;
- sémantique des trois modes contre une base en mémoire ;
- mapping colonnes entrée→sortie (ordre, exclusion, ajouts) ;
- parsing/écriture YAML — dont un test garantissant que **le mot de passe proxy n'est jamais
  sérialisé** (encode l'intention de sécurité) ;
- backoff adaptatif et circuit breaker contre un serveur HTTP mock.

Pas de tests E2E webview (coût/bénéfice défavorable) ; UI vérifiée visuellement.

## Build & distribution

- macOS : `cargo tauri build` local → `.app` zippée ;
- Windows : GitHub Actions (runner Windows) → `.exe` portable ;
- binaires non signés + notice d'ouverture illustrée (Gatekeeper : clic droit > Ouvrir ;
  SmartScreen : « Informations complémentaires > Exécuter quand même »).

## Décisions actées (récapitulatif)

| Question | Décision |
|---|---|
| Utilisateurs | Collègues non techniques |
| Taille | ≤ 20 Mo souple |
| Endpoint | Hybride : batch 1..50 configurable |
| Base SQLite | Globale partagée, dossier données utilisateur |
| Distribution | Non signée + notice |
| Stack | Tauri 2 (Rust) |
| Structure UI | Wizard linéaire (A) |
| Dashboard | Cockpit sombre, ring héros (A) |
| Mapping colonnes | Aperçu direct manipulable (C) |
| Clé API | Persistée YAML + avertissement |
| Credentials proxy | Jamais persistés, mémoire seule |
