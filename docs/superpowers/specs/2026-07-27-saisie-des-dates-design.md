# Saisie des dates du plan de charge — design

Chantier issu d'un constat d'usage sous Windows : « la saisie des dates du plan
de charge ne propose pas de calendrier, juste de la saisie dans le champ. Et
cette saisie ne fonctionne pas. » Maquette validée le 2026-07-27, option A
(`docs/superpowers/maquettes/2026-07-27-saisie-des-dates.html`).

Trois lots, livrables séparément mais issus de la même cause.

## Contexte

L'écran Plan de charge a trois champs `<input type="date">` (`app.js`) : la
fenêtre FUT (`plan-debut`, `plan-fin`, sur `oninput`) et « Ajouter une MEP »
(`plan-mepadd`, sur `onchange`).

### Défaut 1 — le calendrier est dessiné, mais invisible

`styles.css` ne déclare nulle part `color-scheme`. Sa valeur initiale vaut
`normal`, donc le moteur dessine ses propres contrôles en **schéma clair** :
le bouton du sélecteur de date est un glyphe sombre, posé sur `--bg` (#0e1524).
Idem pour les listes déroulantes, les ascenseurs et les cases à cocher.

Constaté sous Windows parce que WebView2 (Chromium) dessine un bouton
calendrier là où WebKit rend d'autres contrôles ; la déclaration manquante,
elle, n'est pas propre à une plateforme.

### Défaut 2 — la frappe notifie des dates absurdes

Un champ date n'a pas de valeur tant qu'il est incomplet — **sauf l'année**,
qui vaut dès son premier chiffre. Mesuré dans Chromium, en tapant `27 07 2026` :

```
input → 0002-07-27
input → 0020-07-27
input → 0202-07-27
input → 2026-07-27
```

`change` se comporte de la même façon. Chacune de ces valeurs est une date
entière et valide aux yeux du navigateur.

Conséquences, par champ :

| Champ | Écouteur | Effet aujourd'hui |
|---|---|---|
| `plan-debut` / `plan-fin` | `oninput` → `planRecalc` (anti-rebond 250 ms) | un aperçu part par chiffre d'année |
| `plan-mepadd` | `onchange` | quatre MEP créées, et le panneau reconstruit **pendant** la frappe |

Côté moteur, le seul garde-fou de `PlanParams::calendrier` est
`fin <= debut` (`plan.rs`) : une fenêtre partant de l'an 2 le passe.
`timeline::timeline` produit **un jour civil à la fois** entre le minimum et le
maximum de toutes les dates du plan. Mesuré, pour un début à l'an 2 et une fin
en 2026 :

```
jours produits = 739409 — JSON 66 Mo
```

66 Mo à sérialiser et à faire traverser l'IPC, puis autant de lignes de tableau
à construire dans `renderPlanParam`. C'est ce qui fige l'application, et fait
perdre les frappes suivantes.

Le champ MEP est pire : son écouteur reconstruit le panneau à chaque
notification, donc **détruit le champ en cours de saisie**. La MEP à l'an 2
reste ensuite dans `plan.meps` et fige tout recalcul tant qu'on ne l'a pas
retirée à la main.

## Lot 1 — Borner les années côté moteur

Le garde-fou de l'interface évite l'aller-retour ; il n'est pas la règle. Un
plan enregistré à la main ou un `runs.csv` fautif n'entrent pas par le champ.

`jour_iso` est le passage obligé des **quatre** sortes de dates du plan (début,
fin, runs, MEP) : la borne s'y pose une seule fois.

```
const ANNEES_PLAUSIBLES: RangeInclusive<i32> = 2000..=2100;
```

Refus, message français nommant le champ et la plage, dans la forme déjà
utilisée par le message voisin :

```
début de fenêtre : la date (0002-07-27) sort des années plausibles (2000-2100)
run 3326 : la date (0202-11-04) sort des années plausibles (2000-2100)
```

Plage assez large pour n'écarter aucun usage réel, assez étroite pour couper
les états de frappe. Bornes **inclusives** — 2000 et 2100 sont dedans.

**Hors périmètre :** aucun plafond sur l'étendue de la timeline en nombre de
jours. La plage d'années suffit à écarter les saisies fautives, et une fenêtre
légitime de plusieurs années reste permise. Décision séparée si le besoin naît.

## Lot 2 — Ne réagir qu'à une date achevée

Prédicat unique dans `app.js`, bornes reprises de `plan.rs` — **à garder
alignées**, comme la parité `active_label` ↔ `ppfActiveTag` :

```
saisieEnCours(v) → v non vide ET année hors [2000, 2100]
```

Un champ **vide** n'est pas une frappe en cours : c'est un effacement, et il
doit continuer de retirer les chiffres de l'écran. C'est la distinction que le
prédicat encode.

- `planRecalc` : sur une frappe en cours, **ne rien faire du tout** — ni calcul,
  ni effacement de `plan.apercu`. Vider l'écran à chaque chiffre ferait
  clignoter le récapitulatif entier ; les chiffres de la dernière fenêtre
  complète restent affichés.
- `plan-mepadd` : sur une frappe en cours, sortir **avant** d'ajouter la MEP et
  avant `renderPlanAside()`. Le geste de fin (vider le champ, reconstruire le
  panneau) n'appartient qu'à la date achevée.

## Lot 3 — Déclarer l'interface sombre

Une ligne, `color-scheme: dark` sur `:root` (`styles.css`).

Elle dit une vérité déjà vraie — l'application n'a qu'un thème. Elle touche
**toute** l'application, pas seulement l'écran du plan : calendriers, listes
déroulantes, ascenseurs, cases à cocher, compteurs. C'est le point qui a été
soumis à validation, contre l'alternative étroite (repeindre le seul glyphe du
calendrier via `::-webkit-calendar-picker-indicator`), écartée parce qu'elle
laisse le panneau du calendrier et les listes s'ouvrir en blanc.

## Tests

| Quoi | Où |
|---|---|
| Les trois états de frappe d'une année sont refusés, en nommant la plage | `plan::tests` |
| Les quatre sortes de dates sont bornées (début, fin, run, MEP) | `plan::tests` |
| Les bornes 2000 et 2100 passent (plage inclusive) | `plan::tests` |
| Une année en cours de frappe ne déclenche aucun calcul — avec témoin qu'une fenêtre complète, elle, calcule | `client/tests/plan_saisie_date.test.js` |
| Une année en cours de frappe n'efface pas les chiffres affichés | idem |
| Un champ vidé, lui, efface bien les chiffres | idem |
| Taper une MEP au clavier n'en ajoute qu'une | idem |
| Le champ MEP ne se vide pas sous les doigts | idem |

Le rendu des contrôles (lot 3) ne se vérifie pas en test : le faux DOM prouve
qu'un nœud existe, jamais qu'il est visible. Il se juge dans l'application, sous
Windows.
