# Super Popaul — étape 2 « Colonnes » en DnD unique avec drop zone (design)

Date : 2026-07-13 · Statut : validé

## Objectif

Faire du glisser-déposer le paradigme unique de l'étape 2 : réordonner,
écarter et réintégrer les colonnes de sortie par le même geste. Le ✕
d'exclusion et le menu « + Ajouter une colonne ⚡ » disparaissent au profit
d'une **drop zone** de colonnes écartées, visible en permanence sous le
tableau d'aperçu.

Variante retenue (validée sur playground comparatif) : **B — colonne
matérialisée en direct**. Le corps du tableau suit l'ordre des en-têtes en
continu pendant tous les drags : une colonne entrante (chip) apparaît avec
ses données dès qu'elle survole les en-têtes (fond bleu translucide), une
colonne sortante disparaît dès qu'elle survole la zone. Le tableau affiché
pendant le drag est exactement le fichier de sortie qu'on obtiendra.

## Décisions actées

- **Suppression totale** du ✕ et du menu « + » : aucun raccourci de secours
  (ni double-clic). Un seul paradigme.
- **Drop zone = bandeau sous le tableau**, bordure pointillée (même langage
  visuel que le dropzone de l'étape 1), chips = libellé seul (`⚡` + bleu
  pour les champs Peppol). Vide, elle affiche un texte d'aide (« Glisse ici
  les colonnes à écarter — et depuis ici pour les réintégrer. »).
- **Nouveau défaut** à la sélection d'un fichier :
  `[...colonnes d'entrée, ⚡ existe, ⚡ CTC-FR]` — `code PA`, `pays PA` et
  `nom PA` démarrent dans la zone. (Changement de forme du fichier de sortie
  par défaut : `pa_code` et `pa_country` sortent du défaut actuel.)
- **Garde « minimum 1 colonne »** : la dernière colonne du tableau refuse
  d'être écartée (`pull` conditionnel Sortable). Un tableau vide n'aurait
  plus de ligne d'en-têtes où re-dropper, et une sortie sans colonnes n'a
  pas de sens. La garde existante « 0 colonnes bloque Suivant » (app.js)
  reste en place, désormais inatteignable.
- **Zone non triable en interne** (`sort: false`) : on drag vers/depuis la
  zone, jamais dedans. Son ordre est canonique à chaque render : champs
  Peppol absents d'abord (ordre `PEPPOL_FIELDS`), puis colonnes d'entrée
  écartées (ordre du fichier). Sans ça, un tri manuel serait silencieusement
  défait au re-render suivant.
- **Annulation** (Échap, drop hors listes) : revert natif Sortable ; le
  re-render d'`onEnd` resynchronise le corps. Rien de plus à coder.
- **Invariant « ≥ 1 colonne » étendu à Rust** : `Config::validate` rejette
  `output.columns` vide (+ test cargo). Sans ça, un YAML `columns: []`
  (constructible avec l'UI actuelle, où tout est ✕-able) chargerait vers un
  tableau sans ligne d'en-têtes — aucune cible de drop, utilisateur coincé.
  Le rendu « 0 colonnes » de `columns.js` et son message deviennent morts et
  sont supprimés.

## Modèle de données — inchangé

`state.config.output.columns` reste la seule source de vérité : liste
ordonnée des colonnes **incluses** (`{source: "input", name}` |
`{source: "peppol", field}`). La drop zone n'est pas stockée : calculée au
render = (champs Peppol + colonnes d'entrée du preview) − incluses — même
formule que l'actuel `renderAddColMenu`. Zéro impact Rust/YAML ; les configs
existantes chargent à l'identique.

## Mécanique DnD (columns.js)

Deux listes Sortable partagent un `group` :

1. la **ligne d'en-têtes** de `#out-preview` (comme aujourd'hui) ;
2. la **drop zone** (`div` de chips).

Options communes : celles du commit précédent (`forceFallback: true`,
`fallbackOnBody: true`, `animation: 250`, `ghostClass`/`fallbackClass`).
`th` et chips portent `data-key` (clé stable : `name` pour input, `field`
pour peppol).

- **Sync virtuelle du corps** : sur l'événement `change` des **deux** listes
  (en inter-listes, Sortable l'émet côté source), le corps est reconstruit
  pour refléter l'ordre courant des en-têtes. Chaque `tr` du corps garde un
  pool `Map(data-key → td)` : clé connue → td du pool (détaché/rattaché),
  clé nouvelle (chip entrante) → td matérialisé (données réelles du preview
  pour une colonne d'entrée, `PEPPOL_SAMPLE` pour un champ Peppol) avec
  classe `.temp` (fond bleu translucide), en-tête absent → ses td restent
  dans le pool, détachés.
- **Commit au drop** : `onEnd` (émis une fois, côté source) →
  `setTimeout(0)` (laisser Sortable clore son cycle) → relire les `data-key`
  de la ligne d'en-têtes → reconstruire `state.config.output.columns` →
  re-render complet (tableau + zone), qui détruit/réinstancie les Sortable.
- **Garde min 1** : `group.pull` de la liste d'en-têtes est une fonction qui
  refuse quand il ne reste qu'un `th`.

## Nettoyage

- `columns.js` : suppression de `renderAddColMenu`, du `rm`/✕ dans
  `makeHeader`, du listener `#btn-add-col`, du rendu « 0 colonnes » ;
  commentaire de tête mis à jour.
- `index.html` : suppression de `#btn-add-col` et `#add-col-menu` ; ajout du
  conteneur de la drop zone.
- `styles.css` : suppression des styles `th .rm` ; ajout `.chip`, `.zone`
  (réutiliser le langage de `#dropzone`), `td.temp`.
- `app.js` : nouveau défaut de colonnes (bloc `headersChanged`).

## Vérification

Harnais HTML (scratchpad, stubs `$`/`h`/`state` + vrais columns.js,
styles.css, vendor) piloté par Playwright, drags souris réels :

- les trois gestes (réordonner / écarter / insérer à un emplacement précis),
  avec vérification du state committé et du DOM après drop ;
- matérialisation en direct (`td.temp` présents pendant le survol) dans les
  deux sens ;
- enchaînements de drags (réinstanciation Sortable après re-render) ;
- garde min 1 (la dernière colonne ne part pas) ;
- annulation Échap (state inchangé, corps resynchronisé) ;
- zone vide (texte d'aide) et nouveau défaut (re-sélection de fichier).

Côté Rust : TDD sur la nouvelle règle de `Config::validate` (rejet de
`columns: []`), `cargo test` + clippy. Validation finale : run Windows
(WebView2), le contexte d'origine du bug DnD étant Windows-spécifique.
