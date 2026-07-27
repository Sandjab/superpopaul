# Jeux de paramètres du plan de charge — design

Chantier issu d'une question d'usage : « si je veux récupérer les mêmes
paramètres de plan qu'un fichier précédent, différent, comment procéder ? »
Maquette à valider (`docs/superpowers/maquettes/2026-07-27-jeux-de-parametres.html`).

## Contexte

### Ce qui marche déjà, et ce qui gêne

Les paramètres suivent **déjà** d'un fichier à l'autre : `ouvrirPlan` restaure
`plan_meta.params_yaml` quel que soit le fichier ouvert, avec un bandeau
d'avertissement quand il diffère (`autre_fichier`).

Deux limites :

1. **Un seul plan est mémorisé.** `plan_meta` porte un `CHECK (id = 1)` : c'est
   toujours le dernier plan généré, jamais un plan antérieur au choix.
2. **Les lignes du plan précédent viennent avec.** À la génération,
   `Preserves::depuis` conserve les gelées (MEP passée), les épinglées (origine
   manuelle) et les retirées, et **sort leurs comptes du nouveau pool**
   (`Preserves::comptes`). Des décisions prises sur l'ancien fichier s'invitent
   donc dans le plan du nouveau.

Le seul contournement connu est du SQL, application fermée
(`DELETE FROM plan_cf;` en gardant `plan_meta` — `charger_plan` s'appuie sur la
méta, pas sur les lignes).

### Ce qui existe déjà et n'est pas à écrire

`PlanParams::vers_yaml` / `depuis_yaml`, avec leurs tests d'aller-retour
(`params_aller_retour_yaml`, `params_yaml_conserve_la_rampe_avec_pilote`,
`params_yaml_conserve_une_rampe_manuelle`). C'est le format déjà persisté. Le
lot ne crée pas un format : il lui donne une porte.

Les compteurs aussi : `PlanApercu` porte déjà `geles`, `epingles`, `retires`.

## Décisions prises

| Question | Réponse retenue |
|---|---|
| Contenu du jeu | **Tout `PlanParams`**, calendrier des runs compris — sinon les volumes d'une rampe manuelle, indexés par n° de run, n'ont plus de sens |
| Chargement vs plan en cours | **Demander seulement s'il y a quelque chose à perdre** |
| Action « repartir de zéro » | **Oui**, indépendante — même mécanique |

## Lot 1 — Enregistrer et charger un jeu

### Point d'entrée

Section **« Jeu de paramètres »** en tête du panneau latéral, au-dessus de
« Colonnes » : un jeu porte tout ce qui suit, calendrier compris. Deux boutons
de largeur égale, « Enregistrer… » et « Charger… », et une ligne d'aide.

Gestes calqués sur les profils (`btn-saveas-cfg` / `btn-load-cfg`) : sélecteur
de fichier du système, filtre YAML (`yaml`, `yml`), pas de répertoire dédié ni
de liste maison.

Une fois un jeu chargé, son nom de fichier remplace la ligne d'aide, en or :
`✓ Chargé depuis « fut-2026-t3.yaml »`. Même rôle que le triptyque du profil —
savoir sur quoi on travaille. **Vit dans la session seulement** : c'est un point
de départ, pas un état à persister.

### Commandes

Deux commandes, sur le modèle de `save_profile` / `load_profile` :

```
plan_params_save(path, params: PlanParams) -> ()
plan_params_load(path)                     -> PlanParams
```

Elles n'écrivent ni ne lisent la base : le jeu est un fichier, le plan est en
base, et les deux ne se mélangent qu'au moment du chargement (lot 3).

### Refus sec

Un jeu illisible est refusé **sans rien modifier** — bandeau d'erreur, panneau
inchangé, comme un profil incompatible. Un jeu à moitié appliqué produirait un
plan que personne ne saurait expliquer.

Le message reprend celui de `depuis_yaml` : `paramètres illisibles : …`.

### Ce qui n'est PAS vérifié au chargement

Aucun contrôle de compatibilité avec le fichier ouvert. Un jeu de paramètres ne
parle pas de colonnes : c'est un calendrier et des réglages, valables sur
n'importe quel fichier. C'est même l'objet du lot. Les bornes d'années de
`jour_iso` s'appliquent, elles, à la génération comme aujourd'hui.

## Lot 2 — Repartir de zéro

Bouton **« Repartir de zéro… »** (`btn-danger`, pleine largeur) en pied de
panneau, sous « Générer le plan » et sa note.

Efface les lignes du plan et **garde les paramètres du panneau** :
`DELETE FROM plan_cf` en laissant `plan_meta`, exactement ce que fait le
contournement SQL. Une commande `plan_reset`.

Ce lot ferme de biais le défaut connu : une ligne ajoutée à la main puis retirée
ne s'efface pas et sort son compte du pool pour de bon. Aucune suppression ligne
à ligne n'est ajoutée — elle contredirait la règle « une ligne retirée n'est
jamais supprimée », qui existe pour expliquer pourquoi un livrable a changé. On
rend seulement le plan **entier** jetable, ce qui est un geste conscient.

## Lot 3 — La confirmation

Elle n'apparaît **que si le plan porte des lignes que la régénération aurait
préservées** — gelées, épinglées ou retirées. Un plan purement automatique, MEP
à venir et sans retouche, se charge sans rien demander : il n'y a rien à perdre.

Deux formes, une seule mécanique :

| Déclencheur | Boutons |
|---|---|
| Chargement d'un jeu | Annuler · Conserver ces décisions · Repartir de zéro |
| « Repartir de zéro… » | Annuler · Repartir de zéro |

La seconde porte en plus un `danger-note` : les comptes retirés perdent leur
motif, et cette trace ne se reconstitue pas.

**Les trois nombres ne se recoupent pas.** Ils suivent l'ordre de
`Preserves::depuis` : une ligne retirée compte comme retirée même si sa MEP est
passée. Les compter autrement donnerait un total supérieur au nombre de lignes.

## Tests

| Quoi | Où |
|---|---|
| Un jeu enregistré puis rechargé rend les mêmes `PlanParams`, calendrier et rampe manuelle compris | `plan::tests` (l'aller-retour YAML est déjà couvert ; ce qui manque est le passage par le disque) |
| Un fichier illisible est refusé, message nommant la cause | `commands` |
| `plan_reset` efface les lignes et **conserve** `plan_meta` — les paramètres se rechargent après | `store::tests` |
| Aucune confirmation quand le plan ne porte aucune ligne préservée | `client/tests/` |
| Confirmation quand il en porte, avec les trois compteurs disjoints | `client/tests/` |
| « Conserver ces décisions » laisse le plan intact ; « Repartir de zéro » l'efface | `client/tests/` |
| Un chargement refusé ne modifie aucun champ du panneau | `client/tests/` |

Le rendu (section en tête, bouton danger en pied, fenêtre de confirmation) ne se
vérifie pas en test : le faux DOM prouve qu'un nœud existe, jamais qu'il est
visible. Il se juge dans l'application.

## Hors périmètre

- Aucun suivi « modifié depuis le chargement » : contrairement au profil, un jeu
  est un point de départ, pas un état à tenir à jour.
- Aucun répertoire de jeux, aucune liste : le sélecteur du système suffit.
- Aucune suppression de ligne à l'unité.
- Aucun changement au comportement actuel de `ouvrirPlan` : le dernier plan
  reste restauré au démarrage de la session.
