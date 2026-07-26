# Rampe manuelle run par run — plan d'implémentation

> **Pour les agents :** SOUS-SKILL REQUIS — utiliser `superpowers:subagent-driven-development`
> (recommandé) ou `superpowers:executing-plans` pour exécuter ce plan tâche par
> tâche. Les étapes sont en cases à cocher (`- [x]`).

**But :** rendre pilotable depuis l'écran la 4ᵉ forme de rampe, `Forme::Manuelle`,
que le moteur sait déjà traiter mais qu'aucune IHM ne renseigne. Ajouter au
passage l'aperçu des volumes en barres, annoncé en v2 et manquant pour les
quatre formes.

**Architecture :** presque tout se joue dans `app.js`. Le moteur est complet et
testé (`construire_rampe` rend les volumes verbatim, un run absent vaut 0), la
sérialisation aussi (`PlanParams` part en YAML dans `plan_meta.params_yaml`).
Une seule correction Rust, défensive. **Aucune logique métier ne descend dans
l'UI** : les volumes sont saisis puis renvoyés tels quels, et tout chiffre
affiché (visé, stock, reliquat) vient de `PlanApercu.timeline`.

**Pile :** Rust (serde), Tauri, JS vanilla sans bundler, CSS maison.

**Maquette validée le 2026-07-26** (variante A retenue, aperçu en barres inclus,
alerte de dépassement demandée, volumes persistés) :
`docs/superpowers/maquettes/2026-07-26-rampe-manuelle.html`

**Spec de référence :** `docs/superpowers/specs/2026-07-25-plan-de-charge-fut-design.md`,
section « v2 » — *rampe manuelle saisie run par run, aperçu en barres*.

**Commandes de référence** (depuis la racine du dépôt, sans `cd`) :

```bash
cargo test  --manifest-path client/src-tauri/Cargo.toml <filtre>
cargo clippy --manifest-path client/src-tauri/Cargo.toml --all-targets
node --test "client/tests/*.test.js"
```

Rappels : **5 warnings clippy préexistants** (`direct.rs`, `resolver.rs`,
`directory.rs`, `commands.rs:88`) — ne pas croire les avoir introduits, ne pas
les corriger ici. Ne **jamais** lancer `node --test` sans motif de fichiers : la
découverte traverse `src-tauri/target/`.

---

## Ce que le moteur fait déjà (ne pas réimplémenter)

| Acquis | Où | Conséquence |
|---|---|---|
| Volumes rendus verbatim, cible ignorée | `plan.rs::construire_rampe` (retour anticipé) | rien à calculer côté UI |
| Run absent de la map → 0, en silence | idem | l'UI doit **lister tous les runs retenus**, un oubli vaut 0 |
| `Forme::Manuelle` sérialisée | `plan.rs`, `#[serde(tag = "forme")]` | la persistance est acquise, il reste à **relire** |
| Stock, placé, reliquat par run | `PlanApercu.timeline[].runs[].detail` | l'alerte et les barres se lisent là, sans recalcul |

## Trois pièges relevés à la maquette

1. **La cible reste active en manuel.** `construire_rampe` l'ignore, mais
   `allouer` s'en sert pour `quotas_par_pa(cible + préservées)` : elle pilote
   toujours la répartition entre plateformes. Ne pas la griser — l'expliquer.
2. **Le pilote, lui, est sans effet** (le code retourne avant `niveau_pilote`),
   mais `rampe_pilote_infaisable` reste appelé et peut émettre un avertissement
   à propos d'un pilote qui ne s'applique pas. Traité en tâche 1 **et** côté JS.
3. **`ouvrirPlan` ne restaure aucun paramètre de rampe** aujourd'hui (ni forme,
   ni raison, ni pilote) : un plan rechargé rouvre en « plate ». Persister les
   volumes sans réparer ça les perdrait au rechargement.

---

## Structure des fichiers

| Fichier | Responsabilité |
|---|---|
| `client/src-tauri/src/plan.rs` | `rampe_pilote_infaisable` : jamais vrai en forme manuelle. |
| `client/src/app.js` | option « Manuelle », champs de volumes, `plan.volumes`, alerte, barres, restauration de la rampe. |
| `client/src/styles.css` | styles des champs de volume et du graphe de barres. |
| `client/tests/plan_aside.test.js` | tests du panneau (existant, à étendre). |
| `client/tests/plan_rampe.test.js` | **neuf** — volumes, bascule, params émis, restauration. |

---

## Tâche 1 — Rust : pas d'avertissement de pilote en forme manuelle

**Fichiers :** `client/src-tauri/src/plan.rs`

Défense en profondeur : la tâche 3 fera envoyer `pilote: null` par l'UI, mais un
YAML persisté par une version antérieure (ou édité à la main) peut porter les
deux, et l'avertissement mentirait alors sur la cause d'un plan trop petit.

- [x] **RED** — test `pilote_infaisable_jamais_signale_en_forme_manuelle` :
      même `Rampe` que `rampe_pilote_infaisable_bascule_sur_la_forme_pure`
      (5 runs, pilote 2×10, cible 25) mais `Forme::Manuelle`, volumes vides →
      `assert!(!rampe_pilote_infaisable(25, 5, &r))`. Vérifier qu'il échoue
      **avant** de coder : aujourd'hui la fonction ne regarde que `rampe.pilote`.
- [x] **GREEN** — retour anticipé `false` sur `Forme::Manuelle`, avec un
      commentaire disant *pourquoi* (le pilote n'a aucun effet dans cette forme).
- [x] `cargo test plan::` vert, clippy inchangé (5 warnings).

## Tâche 2 — JS : les volumes vivent dans l'état, pas dans le DOM

**Fichiers :** `client/src/app.js`, `client/tests/plan_rampe.test.js`

Les champs sont **dynamiques** (un par run retenu) : les laisser au DOM les
exposerait au bug corrigé le 26/07. Ils rejoignent `plan.meps` et `plan.runs`
dans l'état JS.

- [x] **RED** — `client/tests/plan_rampe.test.js` : après bascule sur
      « manuelle » et saisie de deux volumes, un `renderPlanAside()` (déclenché
      par l'ajout d'une MEP) les conserve.
- [x] `plan.volumes = {}` dans l'état initial ; clé = `run_num`, valeur = entier ≥ 0.
- [x] 4ᵉ option du sélecteur de forme : « Manuelle (volume par run) ».
- [x] Bloc « Volumes par run » rendu **seulement** en forme manuelle : une ligne
      par run **retenu** de `plan.apercu.timeline` (`runs` sans `ecart`), dans
      l'ordre chronologique, libellé `<num> · JJ/MM`.
- [x] `oninput` → écrit dans `plan.volumes`, puis `planRecalc()`. Ne **pas**
      re-rendre le panneau à chaque frappe (le champ perdrait le focus).
- [x] Pied de bloc : total saisi, et pool atteignable (somme de `stock_jj` des
      jours couverts — déjà calculé pour la phrase existante sous le graphe JJ).
- [x] Lien « Tout à 0 ».

> **Écart assumé avec la maquette :** le lien « Repartir de la forme linéaire »
> n'est pas implémenté. Il supposerait de répartir une cible côté JS — logique
> métier interdite par les conventions du projet. La bascule pré-remplit déjà
> (tâche 3) ; si le besoin persiste, ce sera un aller-retour `plan_preview`
> dédié, hors de ce lot.

## Tâche 3 — JS : bascule, pré-remplissage, et paramètres émis

**Fichiers :** `client/src/app.js`, `client/tests/plan_rampe.test.js`

- [x] **RED** — basculer sur « manuelle » alors que `plan.volumes` est vide
      recopie les `vise` de l'aperçu courant (le geste réel est « je prends ma
      linéaire et j'ajuste deux runs », pas « je ressaisis six valeurs »).
- [x] **RED** — rebasculer linéaire → manuelle **ne réécrase pas** des volumes
      déjà saisis.
- [x] **RED** — `planParams()` en forme manuelle émet
      `rampe.volumes` non vide **et** `rampe.pilote === null`.
- [x] Supprimer le code mort `if (forme === "manuelle") rampe.volumes = {}`
      (`app.js:1214`) : jamais atteint, et il aurait envoyé une map vide.
- [x] Masquer la case « Pilote prudent » en forme manuelle — masquer ne suffit
      pas, `planParams` doit forcer `null` (la case garde son état).
- [x] Note sous le champ Cible en forme manuelle : elle ne fixe plus le volume,
      elle sert encore de base aux quotas par plateforme.

## Tâche 4 — JS : l'alerte de dépassement

**Fichiers :** `client/src/app.js`, `client/src/styles.css`, `client/tests/plan_rampe.test.js`

L'alerte se lit sur `detail.reliquat > 0` — **pas** sur une comparaison
volume/stock faite dans l'UI : `reliquat = visé + report entrant − placé`, il
tient compte du report, ce qu'un calcul maison raterait.

- [x] **RED** — un run dont le `detail` porte `reliquat > 0` rend son champ avec
      la classe d'alerte et la mention `stock N · R reportés` ; un run à
      reliquat nul ne la porte pas.
- [x] Bordure ambre (`--amber`), jamais rouge : ce n'est pas une erreur, le
      surplus part en report sur le run suivant.
- [x] L'alerte n'apparaît qu'**après** recalcul (elle vient de l'aperçu) : ne
      pas tenter de la produire à la frappe.

## Tâche 5 — JS : aperçu des volumes en barres

**Fichiers :** `client/src/app.js`, `client/src/styles.css`, `client/tests/plan_rampe.test.js`

Vaut pour les quatre formes, pas seulement la manuelle.

- [x] **RED** — une barre par run retenu, dans l'ordre chronologique ; hauteur
      proportionnelle au plus gros `vise` ; barre en ambre si `reliquat > 0`.
- [x] Placé entre la timeline et le graphe du stock par jour de cycle,
      titre « Volumes par run ».
- [x] Cas limite : aucun run retenu → pas de graphe, et surtout pas une division
      par zéro (le graphe JJ a déjà ce garde-fou, `Math.max(1, …)`).

## Tâche 6 — JS : restaurer la rampe à l'ouverture d'un plan enregistré

**Fichiers :** `client/src/app.js`, `client/tests/plan_rampe.test.js`

Manque **préexistant**, révélé par la persistance des volumes : `ouvrirPlan`
restaure début, fin, nombre de MEP, cible et seed — jamais la rampe.

- [x] **RED** — un `plan_load` rendant une rampe manuelle avec volumes rouvre
      l'écran en forme manuelle, champs remplis ; un `plan_load` rendant une
      géométrique de raison 1,8 rouvre en géométrique avec 1,8.
- [x] Poser `plan.volumes` **avant** le premier `renderPlanAside()`, puis les
      valeurs DOM (forme, raison, pilote), puis **re-rendre** : les champs
      conditionnels dépendent de la forme au moment du rendu. La capture des
      valeurs ajoutée le 26/07 garantit que le second rendu conserve le reste.

## Tâche 7 — Vérification finale

- [x] `cargo test` vert (448 attendus : 447 + tâche 1).
- [x] `node --test "client/tests/*.test.js"` vert.
- [x] Passe de mutation sur les bornes neuves — sur ce chantier, **chaque tâche
      Rust avait laissé survivre un mutant** : `reliquat > 0` muté en `>= 0`,
      `plan.volumes` vide muté en « toujours pré-remplir », `pilote: null` muté
      en passe-plat. Un test qui survit à la mutation ne vaut rien.
- [ ] Parcours GUI (le seul filet que les tests JS ne remplacent pas) : saisir
      des volumes, vérifier la timeline et les barres, enregistrer, fermer,
      rouvrir → tout doit revenir.

---

## Hors périmètre

- Les courbes du rapport de plan (`plan_report.rs`) — lot suivant.
- Répartir une cible depuis l'UI (voir l'écart assumé, tâche 2).
- Toute modification de `construire_rampe` : le contrat manuel est déjà celui
  qu'on veut, et il est testé.
