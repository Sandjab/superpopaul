# Timeline calendaire du plan de charge — design

Lot 1 de la v2 du plan de charge. Brainstorm du 2026-07-25, maquette validée le
même jour.

## Contexte

La v1 du plan de charge (commits `4128add..26be482`) affiche les Runs de
Facturation dans une table de neuf colonnes, dans l'onglet *Paramétrage*. Cette
table ne montre que les runs **retenus** : un run écarté disparaît sans laisser
de trace, alors que trois filtres distincts peuvent l'écarter
(`calendrier.rs:181`) — exclusion manuelle, run hors de la fenêtre FUT, run que
la première MEP n'a pas encore précédé. Quand la cible n'est pas atteinte,
l'écran annonce « stock insuffisant sur les jours de cycle des runs retenus »
sans jamais dire quels runs manquent ni quels jours de cycle sont orphelins.

Deux capacités déjà présentes dans le moteur ne sont exposées nulle part :

- `RunParam.exclu` est honoré par `calendrier::runs_utilisables` et couvert par
  un test (`calendrier.rs:304`), mais **aucun code JS ne le renseigne** : on ne
  peut pas exclure un run depuis l'application ;
- `plan::Forme::Manuelle { volumes }` est implémentée dans `construire_rampe`
  (`plan.rs:310`) et `app.js:1214` prépare déjà `rampe.volumes`, mais le
  sélecteur de forme ne propose pas l'option. (Hors de ce lot — noté ici pour
  que le lot suivant parte du bon état.)

## Objectif

Remplacer la table des runs par une **timeline par jour civil** qui rend
lisibles, au même endroit : les runs et leurs chiffres, les runs écartés et leur
motif, les jalons du calendrier (MEP, bornes de fenêtre), et les jours chômés.
Y adjoindre la distribution du pool par jour de cycle, qui répond à la question
« où sont les comptes hors d'atteinte ».

## Décisions validées (brainstorm 2026-07-25)

1. **La timeline remplace la table des runs** ; elle ne s'y ajoute pas. Les deux
   montrent le même objet, et la lecture par jour civil est strictement plus
   riche. Deux vues des mêmes chiffres finiraient par diverger.
2. **Forme : table par jour, colonnes alignées.** Écarté : la grille calendaire
   à sept colonnes (une case ne peut pas loger cinq nombres, et comparer les
   runs exigerait un clic par run) ; écarté aussi : le repli des jours creux
   (sur le calendrier réel `runs.brm.csv`, 54 runs sur 89 jours — 60 % des jours
   portent un run, replier ne gagnerait qu'une quinzaine de lignes contre un
   état d'ouverture à gérer).
3. **Fériés : décor pur, calculés seulement.** Les onze fériés nationaux
   français, calculés. Pas de saisie de fériés supplémentaires (le
   `extra_holidays` de peppolstat est chargé et validé mais lu par aucun calcul),
   et pas d'avertissement quand un run ou une MEP tombe un jour chômé.
4. **Stock par jour de cycle : graphe de 31 barres**, sous la timeline. Hauteur
   = comptes du pool sur ce jour de cycle, couleur = couvert ou non par un run
   retenu. Donne à la fois le diagnostic et la distribution, qui informe le choix
   de la fenêtre et des runs.
5. **La timeline est construite entièrement côté Rust.** Le JS ne fait que
   rendre des lignes. Conforme à « l'UI n'a aucune logique métier » ; le motif
   d'écart d'un run et son rattachement à une MEP sont du métier. Écarté :
   dérouler les jours côté JS, là où peppolstat a dû forcer l'UTC pour éviter
   les décalages de fuseau.
6. **La case « exclure » vit dans la timeline**, une par run, y compris sur un
   run déjà écarté (un run hors fenêtre peut y rentrer si la fenêtre s'élargit).
7. **Étendue** : de `min(première date de run, début de fenêtre)` à
   `max(dernière date de run, fin de fenêtre)`. Les runs hors fenêtre restent
   donc visibles avec leur motif. Sans aucun run chargé, l'étendue se réduit à
   la fenêtre.
8. **Pas de plafond de lignes**, contrairement à l'onglet 2 : l'étendue est
   bornée par le calendrier importé, de l'ordre de 90 jours en pratique.

## Architecture

Un module neuf, `timeline.rs`, plutôt qu'un ajout à `plan.rs` (2 078 lignes) :
la responsabilité est distincte et le projet range une responsabilité par
module. Il dépend de `calendrier` (`RunFacturation`, `feries`) et de `plan`
(`DetailRun`), et ne renvoie que des données — aucun accès au store, aucune E/S.

Le calcul des fériés va dans `calendrier.rs` : c'est du calendrier pur, sans
rapport avec le plan. Le stock par jour de cycle reste dans `plan.rs` : il porte
sur le pool, pas sur le calendrier.

## Modèle de données

```rust
// calendrier.rs
/// Les onze fériés nationaux français de l'année, triés, avec leur nom.
/// Pas de particularisme d'Alsace-Moselle (parité peppolstat).
pub fn feries(annee: i32) -> Vec<(NaiveDate, &'static str)>;
```

Huit dates fixes — Jour de l'an, Fête du Travail, Victoire 1945, Fête
nationale, Assomption, Toussaint, Armistice, Noël — et trois dates mobiles
dérivées de Pâques (computus de Meeus, forme grégorienne) : `+1` lundi de
Pâques, `+39` Ascension, `+50` lundi de Pentecôte. Les trois mobiles sont
nommées, là où peppolstat les laissait sous un « férié » générique.

```rust
// timeline.rs
pub enum Jalon { DebutFenetre, FinFenetre, Mep(usize) }   // Mep : rang, 1-indexé

/// Pourquoi un run ne compte pas. Miroir de `calendrier::runs_utilisables`.
pub enum Ecart { Exclu, HorsFenetre, MepNonPassee, AucuneMep }

pub struct RunJour {
    pub num: String,
    pub jjs: Vec<u8>,
    pub exclu: bool,
    pub ecart: Option<Ecart>,
    /// Les cinq chiffres — présents si et seulement si `ecart` est `None`.
    pub detail: Option<DetailRun>,
}

pub struct JourTimeline {
    pub date: String,              // ISO, comme le reste de la charge utile
    pub jour_semaine: &'static str, // « lun » … « dim »
    pub weekend: bool,
    pub ferie: Option<&'static str>,
    /// Plusieurs jalons peuvent tomber le même jour (une MEP le jour de la
    /// fin de fenêtre, par exemple).
    pub jalons: Vec<Jalon>,
    /// Une liste, pas un `Option` : `parse_runs_csv` refuse deux runs à la
    /// même date (`calendrier.rs:104-111`), mais `PlanParams::calendrier` ne
    /// le revérifie pas en reconstruisant les runs depuis les paramètres
    /// persistés — et c'est ce chemin-là qui alimente l'écran. Un run perdu
    /// en silence est exactement ce que ce lot corrige.
    pub runs: Vec<RunJour>,
}

pub fn timeline(
    runs: &[RunFacturation],   // tous les runs importés, pas seulement les retenus
    debut: NaiveDate,
    fin: NaiveDate,
    meps: &[NaiveDate],
    details: &[DetailRun],
) -> Vec<JourTimeline>;
```

`RunJour` porte `num` et `jjs` en propre parce qu'un run écarté n'a pas de
`DetailRun` : la redondance avec `DetailRun.run_num` / `.jjs` sur les runs
retenus est assumée, le rendu lit toujours les champs de `RunJour`.

`Ecart::MepNonPassee` couvre deux cas que le filtre `r.date > premiere_mep`
traite ensemble : un run antérieur à la première MEP, et un run tombant **le
jour même** de cette MEP. Le libellé retenu — « la première MEP n'est pas
encore passée » — est juste dans les deux cas, là où « avant la première MEP »
serait faux pour le second.

`Ecart::AucuneMep` en est séparé, bien que `runs_utilisables` les traite du
même geste (retour anticipé quand `meps` est vide). Les deux situations
n'appellent pas la même action : décaler une date d'un côté, créer une MEP de
l'autre. Et « aucune MEP définie » n'est pas un cas limite — c'est l'état
initial de l'écran, avant toute saisie. La distinction est tranchée côté Rust
parce que le rendu ne porte aucune logique métier.

**Priorité entre motifs**, imposée par le fait que `runs_utilisables` enchaîne
ses conditions par `&&` — un run peut en échouer plusieurs, `Option<Ecart>`
n'en garde qu'un : `Exclu` > `HorsFenetre` > (`AucuneMep` | `MepNonPassee`,
mutuellement exclusifs). `Exclu` prime parce que c'est le seul motif que
l'utilisateur pilote depuis l'écran ; `HorsFenetre` prime sur les motifs de MEP
parce qu'un run hors fenêtre le reste quoi qu'on fasse aux MEP, et que le motif
affiché est lu comme un conseil d'action.

```rust
// plan.rs
pub struct StockJJ { pub jj: u8, pub comptes: usize, pub couvert: bool }

/// Distribution du pool sur les 31 jours de cycle, et couverture par les runs
/// retenus. Toujours 31 entrées, y compris les jours de cycle vides.
pub fn stock_par_jj(pool: &[CfCandidat], retenus: &[RunFacturation]) -> Vec<StockJJ>;
```

```rust
// commands.rs — PlanApercu
- pub details: Vec<crate::plan::DetailRun>,
+ pub timeline: Vec<crate::timeline::JourTimeline>,
+ pub stock_jj: Vec<crate::plan::StockJJ>,
```

`details` n'a qu'un seul consommateur, la table que ce lot remplace : le rapport
HTML (`plan_report.rs`) travaille sur `lignes`, pas sur `details`. Le champ part
donc sans laisser de dette.

## IHM

Dans l'onglet *Paramétrage*, `renderPlanParam` remplace la table de neuf
colonnes par la timeline. Le rendu est une projection directe de
`apercu.timeline`, sans décision : une ligne d'en-tête quand le mois change,
puis, pour chaque jour, une ligne par jalon **avant** la ligne du jour. Un jalon
et un run tombant le même jour occupent donc deux lignes — c'est ce qui place le
motif d'écart juste sous le jalon qui le cause.

Colonnes : Jour · Run · Jours facturés · Visé · Report · Stock · Placé ·
Reliquat · exclure. Sur un run écarté, les cinq colonnes chiffrées cèdent la
place au motif, qui occupe leur largeur — cinq cases vides seraient du bruit.

La case « exclure » bascule `plan.runs[i].exclu` puis appelle `planRecalc()`.
Le champ circule déjà dans la charge utile existante : aucun code Rust à
toucher.

Le graphe de stock par jour de cycle suit, en 31 barres, avec une légende
chiffrée (comptes éligibles · atteignables · hors d'atteinte).

Construction par le helper `h()`, jamais d'`innerHTML` — les numéros de run et
les raisons sociales viennent du CSV, entrée non fiable.

## Marquage visuel

Conforme à l'identité « Bleu nuit & or » du client : ni or ni orange sur les
états, l'or restant l'accent d'action et l'orange l'avertissement.

| État | Traitement |
|---|---|
| MEP | Bande pleine largeur, liseré plein et dégradé `--green-later` — un jalon coupe le calendrier, il ne qualifie pas un jour |
| Bornes de fenêtre | Filet en pointillés `--border`, libellé en capitales `--muted` — plus faible que la MEP : la fenêtre est un cadre, la MEP un événement |
| Jour de run retenu | Fond `rgba(43,55,82,.28)`, date en `--fg` gras |
| Run écarté | Pas de fond, date en `--muted`, motif en `--muted` |
| Week-end, férié | Aucune couleur propre : opacité réduite et libellé en italique — le décor ne doit pas attirer l'œil plus que les runs |
| Report, reliquat | `--amber` gras, classe `.carry` existante, inchangée |
| Stock par JJ | `--green` si un run retenu couvre le jour, `--red` sinon |

Tous les chiffres en `font-variant-numeric: tabular-nums`, comme
`table.plan-data`.

## Tests (TDD, Rust)

`calendrier::feries`

- Les onze dates d'au moins deux années sont figées en dur. 2026 est l'une
  d'elles : Pâques le 5 avril, donc lundi de Pâques le 6 avril, Ascension le
  14 mai, lundi de Pentecôte le 25 mai. Les dates de la seconde année sont à
  établir depuis une source de référence externe au moment d'écrire le test —
  jamais depuis l'implémentation qu'elles servent à vérifier.
- Une année bissextile, pour la propagation du décalage après février.

`timeline::timeline`

- Aucun jour manquant entre les deux bornes de l'étendue, et l'étendue déborde
  bien la fenêtre quand un run tombe au-delà.
- **Un run hors fenêtre reste visible avec son motif.** Le pourquoi : sans lui,
  la cible non atteinte reste inexplicable — c'est le défaut de la v1 que ce lot
  corrige.
- **Un run tombant le jour même de la première MEP porte `MepNonPassee`.** Le
  pourquoi : le filtre est strict (`>`), et c'est le cas limite que le libellé
  doit couvrir sans mentir.
- **Sans aucune MEP, le motif est `AucuneMep`, pas `MepNonPassee`.** Le
  pourquoi : les deux motifs sont lus comme des conseils d'action opposés, et
  c'est l'état initial de l'écran.
- **Les runs sans écart sont exactement ceux que retient
  `calendrier::runs_utilisables`.** Le pourquoi : `timeline` rejoue ce filtre à
  la main faute de pouvoir l'appeler — il ne rend pas de motif — donc deux
  implémentations de la même règle cohabitent. L'échantillon du test doit poser
  des runs **sur les bornes**, sinon il ne peut pas voir les bornes bouger.
- Un run exclu à la main porte `Exclu`, et le porte même s'il est aussi hors
  fenêtre — l'exclusion est le seul motif que l'utilisateur contrôle, elle prime
  à l'affichage.
- Un run retenu porte son `DetailRun` ; un run écarté n'en porte aucun.
- Plusieurs jalons le même jour sont tous rendus.
- Sans aucun run, l'étendue se réduit à la fenêtre et aucune ligne ne porte de
  run.

`plan::stock_par_jj`

- Les 31 entrées sont présentes, y compris pour un jour de cycle sans compte.
- **Un jour de cycle qu'aucun run retenu ne couvre ressort comme non couvert.**
  Le pourquoi : c'est la seule façon de savoir où sont les comptes hors
  d'atteinte.
- **Exclure un run retire la couverture de ses jours de cycle.** Le pourquoi :
  l'effet d'une exclusion sur le pool atteignable doit être visible
  immédiatement, sinon l'exclusion se fait à l'aveugle.

## Hors-scope

- Rampe manuelle run par run et aperçu en barres — lot suivant, moteur déjà
  prêt (`Forme::Manuelle`).
- Courbes du rapport HTML (cumulée, charge par jour civil).
- Export/import des paramètres — `PlanParams::vers_yaml` / `depuis_yaml`
  existent déjà, il manque le point d'entrée.
- Fériés supplémentaires saisis à la main, et avertissement sur un run ou une
  MEP tombant un jour chômé (décision 3).
- Pose d'une MEP par clic sur la timeline : les MEP restent saisies dans le
  panneau latéral.
- L'onglet 2, le rapport de run, la CLI, le serveur : inchangés.

## Fichiers touchés

| Fichier | Nature |
|---|---|
| `client/src-tauri/src/timeline.rs` | **nouveau** — `Jalon`, `Ecart`, `RunJour`, `JourTimeline`, `timeline()` |
| `client/src-tauri/src/lib.rs` | déclaration du module |
| `client/src-tauri/src/calendrier.rs` | `feries()` + computus de Meeus |
| `client/src-tauri/src/plan.rs` | `StockJJ`, `stock_par_jj()` |
| `client/src-tauri/src/commands.rs` | `PlanApercu` : `details` → `timeline`, `+ stock_jj` |
| `client/src/app.js` | `renderPlanParam` : timeline + graphe JJ + case « exclure » |
| `client/src/styles.css` | styles de la timeline et des barres de jour de cycle |
