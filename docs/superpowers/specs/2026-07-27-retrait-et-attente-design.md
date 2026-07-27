# Annulation du retrait & indication de traitement — design

Chantier issu de la revue de code du 2026-07-27, qui a relevé une promesse non
tenue à l'écran, et d'une demande de l'utilisateur sur les temps d'attente.
Maquette validée le même jour
(`docs/superpowers/maquettes/2026-07-27-retrait-et-attente.html`).

Deux lots **indépendants**, livrables séparément.

## Contexte

### Lot 1 — le retrait est annoncé annulable, et ne l'est pas

`ouvrirRetrait` affiche « Les lignes restent consultables via le filtre
« retiré » et ne seront pas replacées par une régénération. **Le retrait est
annulable.** » (`app.js:1988`).

La commande existe (`commands::plan_annuler_retrait`), elle est enregistrée
(`lib.rs:104`), le moteur la couvre (`plan::annuler_retrait`) — mais **aucun
appel ne part du frontend** : `grep annuler_retrait client/src/` ne rend que la
phrase ci-dessus. Un retrait est donc définitif à l'écran, contrairement à ce
qu'on y lit.

### Lot 2 — les actions longues ne disent rien

Toutes les commandes du plan refont un scan CSV complet et une jointure SQLite
(`plan_entrees_from_scan`). Aucune n'émet d'avancement. Aujourd'hui :

| Action | Retour visuel |
|---|---|
| `plan_generate` (Générer le plan) | aucun |
| `plan_preview` (aperçu, anti-rebond 250 ms) | aucun |
| `plan_candidats_run` (« + Ajouter » d'un run) | aucun — la fenêtre apparaît après |
| `plan_ajouter` / `plan_deplacer` / `plan_retirer` | aucun |
| `plan_rapport` | **bouton désactivé + libellé « … »** (`app.js:1810`) |

Le dernier est le seul à faire quelque chose, et son libellé ne nomme pas
l'action. Ailleurs dans l'application, la convention est établie et parlante :
`out.textContent = "test en cours…"` (`app.js:679`),
`"calibration en cours…"` (`app.js:783`), plus `setDirBusy` / `setPpfBusy` qui
désactivent les déclencheurs et montrent une barre — barre alimentée par un
avancement réel, que les commandes du plan n'ont pas.

## Lot 1 — Réactiver un compte retiré

### Point d'entrée

Un bouton **« Réactiver *n* retiré(s)… »** dans `.plan-selbar`, aux côtés de
« Déplacer vers un run… » et « Retirer… ».

Il n'apparaît que si la sélection contient au moins une ligne retirée. Sur une
sélection **mixte**, il n'agit que sur les retirées et **le dit dans son
libellé** — plutôt que de refuser la sélection : filtrer sur « retiré » n'est
pas toujours le geste, et proposer une action qui échoue ensuite serait hostile.

Rejeté : une action par ligne. Réactiver trente comptes après un incident
résolu est le cas réel ; le filtre « retiré » existe déjà pour les rassembler.

### Confirmation

Systématique, même sans MEP gelée — un seul chemin à maintenir, et c'est
l'occasion de prévenir de la perte du motif.

Elle contient :

1. ce qui va se passer — les comptes redeviennent livrables et repartiront dans
   les fichiers de leur MEP ; une régénération pourra les replacer ailleurs ;
2. l'avertissement MEP gelée **quand il s'applique**, dans les mêmes termes que
   `ouvrirRetrait` : les deux gestes changent un fichier déjà transmis ;
3. « ⚠ Le motif du retrait sera perdu. »

### Ce que le lot ne fait pas

`annuler_retrait` remet `retire` à `None` : le motif est **effacé**. Le
conserver demanderait une table d'historique que rien ne réclame. La
confirmation le dit, faute de pouvoir l'éviter — c'est ce qui justifie qu'elle
soit systématique.

Aucun changement côté Rust : `plan::annuler_retrait` et
`commands::plan_annuler_retrait` existent et sont testés.

## Lot 2 — Indication de traitement

### Actions explicites : le bouton parle et se désactive

`« Générer le plan » → « Génération en cours… »`, bouton grisé, rétabli à la fin
**quoi qu'il arrive** — succès comme erreur.

C'est la convention de l'API, généralisée depuis l'embryon inline du bouton
rapport. Un helper unique porte le patron :

```js
occupe(bouton, "Génération en cours…", async () => { … })
```

Il sert : Générer le plan, Rapport du plan, « + Ajouter » d'un run, et les
boutons de validation des fenêtres Ajouter / Déplacer / Retirer / Réactiver.

Deux effets, pas un : l'indication est **là où le regard vient de cliquer**, et
le bouton désactivé fait **garde de ré-entrance** — plus de double génération.

**Les fenêtres de retouche restent ouvertes jusqu'à la fin du rechargement.**
Aujourd'hui `closeModal()` s'exécute avant `rechargerRecap()` : la fenêtre
disparaît, puis l'écran reste figé sans rien pour l'expliquer. La fermeture
passe après.

Rejeté : une barre de progression sur l'écran du plan. Aucune de ces commandes
n'émet d'avancement — la barre serait décorative, là où celles de l'annuaire et
du PPF sont alimentées.

Rejeté : un voile bloquant. Étranger au reste de l'application, et le bouton
désactivé suffit à empêcher le double déclenchement.

### Aperçu automatique : les chiffres se marquent périmés

L'aperçu se relance à chaque changement de paramètre. Un indicateur franc y
clignoterait ; l'absence d'indicateur laisse un écran qui paraît figé.

Pendant le recalcul :

- les zones portant des chiffres d'aperçu prennent une classe `.perime`
  (`opacity: .42`) ;
- un « calcul… » discret apparaît à **place fixe** dans `#plan-foot`, sans rien
  pousser.

Le problème n'est pas qu'une tâche tourne, c'est que **les chiffres lus ne
correspondent plus à ce qu'on vient de saisir**. L'atténuation le dit ; une
barre de progression laisserait croire l'inverse.

**L'atténuation épargne les champs de saisie** du panneau latéral : c'est la
frappe qui déclenche le recalcul, griser ce qu'on est en train de régler serait
absurde — et `suivreApercuDansLePanneau` protège déjà la saisie en cours.

## Tests

Faux DOM (`client/tests/`), qui exécute le vrai `app.js`. Il prouve le câblage,
jamais le rendu : la validation visuelle reste un parcours en application.

Lot 1 :
- le bouton n'apparaît pas sans ligne retirée dans la sélection ;
- sur sélection mixte, le libellé compte les retirées seules ;
- la validation n'envoie que les CF retirés à `plan_annuler_retrait` ;
- l'avertissement MEP gelée n'apparaît que si une ligne gelée est concernée.

Lot 2 :
- pendant l'appel, le bouton est désactivé et son libellé a changé ;
- après succès **et après erreur**, il retrouve libellé et état ;
- un second clic pendant le traitement ne déclenche pas de second appel ;
- la fenêtre de retouche ne se ferme qu'après le rechargement ;
- `.perime` est posée pendant le recalcul et retirée après ;
- les champs de saisie du panneau ne prennent jamais `.perime`.

## Hors périmètre

- Historique des retraits (le motif reste effacé à la réactivation).
- Avancement réel des commandes du plan (aucune ne l'émet).
- Les écrans hors Plan de charge : `analyze_input`, `export_report`,
  `generate_output` gardent leur comportement actuel.
