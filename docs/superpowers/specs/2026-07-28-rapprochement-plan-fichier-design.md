# Rapprochement du plan avec un nouveau fichier de comptes — design

Un plan est établi à partir d'un fichier de comptes de facturation F1. Un mois
plus tard, F2 arrive : mêmes comptes pour l'essentiel, mais des jours de cycle
qui ont bougé, des comptes disparus, des comptes devenus inéligibles côté CTC
Peppol ou côté annuaire PPF.

Le besoin est de **mettre le plan à jour en le perturbant le moins possible** :
retirer ce qui n'est plus éligible, déplacer ce qui a changé de jour de cycle,
et ne toucher à rien d'autre.

## Ce que l'application sait faire aujourd'hui

Trois briques sont déjà en place :

- le **changement de fichier est détecté** — `rapport_au_fichier`
  (`commands.rs:1224`) rend quatre états depuis la v1.4.1, dont
  « même nom, contenu différent » ;
- l'**éligibilité est recalculée en continu, jamais figée** — `etat_de`
  (`commands.rs:1339`) confronte chaque ligne du plan au fichier courant et
  rend `eligible` · `ctc_non_pret` · `ppf_non_utilisable` ·
  `absent_du_fichier`, filtrables dans le récap (`app.js:2087`) ;
- le **retrait en lot existe** (`plan_retirer`) et survit à la régénération
  via `Preserves.retirees`.

## Ce qui manque, et pourquoi ça bloque

### 1. Il n'existe aucune mise à jour incrémentale

Le seul chemin de recalcul est `plan::regenerer` (`plan.rs:778`) : tout ce qui
n'est ni gelé, ni épinglé, ni retiré est **entièrement ré-alloué**. Or
`trier_par_priorite` (`plan.rs:553`) classe sur `in_directory` et `resolved_at`,
qui bougent entre F1 et F2, et le pool change de taille. Des comptes
strictement inchangés changent donc de run, voire sortent du plan. C'est
l'inverse exact du besoin.

Le seul rempart existant est l'épinglage (`Origine::Manuel`), qui s'obtient
ligne à ligne — inutilisable à l'échelle.

### 2. Le jour de cycle des lignes préservées est figé, et jamais confronté au fichier

`LignePlan` est **auto-porteuse** par conception (`plan.rs:484-507`) : `jj`,
`pa`, `in_directory` sont ceux de F1, pour que le gel survive à un changement
de fichier. `plan_lignes` recalcule bien `etat`, mais affiche `l.jj`, l'ancien
(`commands.rs:1378`). Un changement de jour de cycle est donc **totalement
invisible**, et la ligne reste accrochée à un run qui ne couvre plus son vrai
jour.

### 3. Le déplacement manuel ne peut pas le rattraper

`plan::deplacer` valide contre `plan[i].jj`, l'ancien (`plan.rs:899`), et
l'IHM ne propose que les runs compatibles de ce même ancien jour
(`app.js:2183`). Un compte passé du jour 5 au jour 12 : le run traitant le 12
est refusé (« ne traite pas le jour 5 »), et le seul run acceptable conserve un
jour faux. Retirer puis ré-ajouter ne marche pas non plus — `retirer` ne
supprime pas la ligne, et `ajouter` refuse tout compte déjà présent sans
exclure les retirées (`plan.rs:867`). Le seul recours actuel est `plan_reset`,
qui jette gel, épingles et retraits.

## Périmètre

Ce que le rapprochement détecte et ce qu'il en fait :

| Écart | Action |
|---|---|
| Éligibilité perdue (CTC non prêt, PPF non utilisable) | retrait |
| Compte disparu du fichier | retrait |
| Jour de cycle changé | déplacement vers un run compatible |
| Plateforme changée | rafraîchissement du champ, sans ré-allocation |
| Adressage ou raison sociale changés | rafraîchissement silencieux |
| Tout le reste | intact |

**Aucun ajout.** Les comptes éligibles absents du plan — nouveaux dans F2, ou
présents et devenus éligibles — restent dehors. Les ajouter rouvrirait
l'allocation, c'est-à-dire exactement la perturbation qu'on cherche à éviter.
Le geste manuel « + Ajouter » depuis un run reste disponible pour les cas
ponctuels.

**Un changement d'adressage n'est pas une action.** L'adressage porte la
jointure PPF et CTC : s'il change et casse l'éligibilité, le compte tombe dans
le premier cas du tableau. S'il change sans la casser, seule la valeur portée
par la ligne est rafraîchie.

### Lignes gelées

Une ligne est **gelée** quand sa MEP est passée : son fichier a été transmis,
et les fichiers sont cumulatifs.

- **Retrait : oui**, mais présenté à part, avec l'avertissement que ça réécrit
  un fichier déjà transmis. C'est la sémantique que le code porte déjà —
  `plan::retirer` autorise explicitement le retrait sur MEP gelée
  (`plan.rs:915`, « c'est un besoin réel : on sait qu'un compte va échouer »)
  et `ecrire_fichiers_mep` exclut les retirées de tous les fichiers, y compris
  sur une MEP passée.
- **Déplacement : jamais.** Rien nulle part n'autorise à sortir un compte d'un
  lot livré pour l'insérer dans un autre. L'écart est signalé, pas traité.

## Prérequis : la fraîcheur des sources

`plan_entrees_from_scan` (`commands.rs:782`) relit la base **à chaque appel** :
`load_map` pour les résolutions CTC, `ppf_flags` pour l'annuaire PPF,
`directory_present` pour l'annuaire Peppol. Charger un annuaire neuf et
relancer une résolution **avant** de rapprocher suffit donc : le rapprochement
en tient compte sans qu'aucun code ne le pilote.

Deux limites doivent être dites à l'écran plutôt que contournées.

### L'éligibilité PPF ne peut que se gagner

`ingest_ppf` (`store.rs:456`) fait
`INSERT … ON CONFLICT(identifiant, motif) DO UPDATE SET pdp_fictive` : aucune
suppression, et le test qui garde ce comportement s'appelle
`ppf_upsert_cumulatif_conserve_les_motifs`. Un identifiant **sorti** de
l'annuaire, ou passé d'un motif actif à un motif inactif, conserve sa ligne et
**reste utilisable**. Charger un annuaire neuf ne fera jamais basculer un
compte vers « PPF non utilisable » ; seul `reset_ppf` (`store.rs:583`, déjà
exposé, `app.js:1193`) puis rechargement le peut.

L'utilisateur a confirmé que chaque livraison d'annuaire est **complète** — le
cumul est donc inadapté à son usage et produit des faux positifs d'éligibilité
qui grandissent à chaque chargement. **La correction du chargement PPF est un
lot séparé.** Ce chantier-ci se contente de refuser de mentir : quand
`ppf_files` en compte plus d'un, le rapprochement affiche que la perte
d'éligibilité PPF n'y est pas détectable, et renvoie vers « vider puis
recharger ».

Asymétrie à connaître : l'annuaire **Peppol** est remplaçant
(`directory_est_recreee_a_chaque_chargement`, `store.rs:930`), l'annuaire PPF
est cumulatif. Deux annuaires, deux sémantiques opposées.

### Résoudre « les comptes du plan » n'existe pas

`compute_todo` (`modes.rs:31`) travaille sur les adressages uniques du
**fichier d'entrée**. Résoudre F2 couvre les comptes du plan, sauf les disparus
— qui n'ont plus d'adressage à résoudre. Le mode importe : `Reprise` ne
retente que les échecs, il faut `Full` ou un `Refresh` court pour rafraîchir un
verdict valide mais périmé.

Nuance : le statut CTC est **temporel** — `ctc_status(r, now)` évalue les dates
stockées contre maintenant. Un compte bascule donc tout seul avec le temps,
sans nouvelle résolution.

## Le geste

Un bouton **« Rapprocher avec le fichier ouvert »** calcule le diff **sans
rien écrire** et l'affiche groupé par nature d'écart. Lecture, puis
**application en bloc** : pas de décochage ligne à ligne.

C'est la paire que le projet pratique déjà — `plan_preview` calcule le vrai
plan sans l'écrire, `plan_generate` écrit.

Rejeté : l'application directe avec compte rendu après coup. Un déplacement
**n'est pas annulable**, contrairement à un retrait ; découvrir après coup que
180 comptes ont changé de run n'aurait pas de marche arrière hors régénération
complète, précisément ce qu'on fuit.

Rejeté : le signalement seul, avec action manuelle par les outils existants.
Il faudrait de toute façon débloquer `deplacer`, il n'y a pas de « tout
sélectionner », l'affichage plafonne à 500 lignes, et le run cible resterait à
choisir à la main alors que le calcul le connaît.

## Modèle

Un rapprochement est une **liste d'écarts**, un par compte concerné. Chaque
écart porte ce qui a changé et ce qu'on en fait, **séparément** : la même
nature ne donne pas la même action selon que la ligne est gelée.

```rust
pub enum Nature {
    EligibilitePerdue { avant: String, apres: String },
    DisparuDuFichier,
    JourChange { avant: u8, apres: u8 },
    PlateformeChangee { avant: String, apres: String },
}

pub enum Action {
    Retirer { motif: String },
    Deplacer { run_num: String, run_date: NaiveDate, mep_id: usize, mep_date: NaiveDate },
    Rafraichir,
    Signaler,
}

pub struct Ecart {
    pub cf: String,
    pub nature: Nature,
    pub action: Action,
    pub gelee: bool,
}

pub struct Rapprochement {
    pub ecarts: Vec<Ecart>,
    /// Lignes qu'aucun écart ne concerne. Une ligne dont seuls l'adressage ou
    /// la raison sociale ont changé en fait partie : ces champs sont
    /// rafraîchis sans produire d'écart (voir plus bas).
    pub inchangees: usize,
    /// Avertissements **dérivés du calcul** : répartition par plateforme
    /// modifiée, ampleur des retraits. Ceux qui dépendent de l'état de la base
    /// — annuaire PPF cumulatif — sont ajoutés par la commande, qui seule y a
    /// accès.
    pub avertissements: Vec<String>,
}
```

L'empreinte SHA-256 du fichier **ne fait pas partie** de cette structure :
`calculer` est pure et ne lit aucun fichier. Elle est jointe par la commande,
dans une enveloppe qui lui est propre :

```rust
// commands.rs
#[derive(Serialize)]
pub struct RapprochementVue {
    pub rapprochement: crate::rapprochement::Rapprochement,
    pub empreinte: String,
    /// Séparé des avertissements du calcul : ceux-là décrivent ce que le
    /// rapprochement va faire, celui-ci prévient qu'il est **incomplet**.
    pub annuaire_incomplet: Option<String>,
}
```

L'avertissement d'annuaire cumulatif ne rejoint donc **pas**
`Rapprochement.avertissements`, qui ne porte que ce qui dérive du calcul. Il
n'est pas du même ordre : les autres annoncent des conséquences, celui-ci
prévient que le résultat peut être muet là où il devrait parler. L'écran le
rend au-dessus des autres, dans un registre visuel plus grave.

### Une ligne, un écart

Un compte peut cumuler les changements. L'ordre de résolution est explicite,
comme celui de `Preserves::depuis` — **retrait > déplacement >
rafraîchissement**. Inutile de déplacer un compte qu'on retire.

L'examen, ligne par ligne du plan, prend la première branche qui s'applique :

1. déjà retirée → ignorée
2. absente du fichier → `Retirer`
3. plus éligible (CTC non prêt ou PPF non utilisable) → `Retirer`
4. jour de cycle différent → `Deplacer` ; `Signaler` si la ligne est gelée ou
   si aucun run ne convient
5. plateforme différente → `Rafraichir`
6. sinon → inchangée

**Le rafraîchissement de l'adressage et de la raison sociale est à part.** Il
s'applique à **toute** ligne conservée dont le compte est encore au fichier,
quelle que soit la branche retenue, et ne produit **aucun écart** : sans effet
sur le placement, il n'a rien à faire valider. Une ligne dont seuls ces champs
ont bougé est donc comptée dans `inchangees`. L'écran en donne le nombre, sans
détail ligne à ligne.

### Le choix du run cible

Règle de moindre perturbation. Parmi les runs **utilisables** qui couvrent le
nouveau jour de cycle, celui dont la MEP est **la plus proche** de celle où la
ligne se trouve déjà — distance nulle pour la MEP courante, qui l'emporte donc
d'office et laisse le compte dans son lot. La date de run départage les
ex æquo.

**Double garde temporelle**, et la seconde n'est pas redondante :

1. jamais un run déjà passé ;
2. jamais un run dont la **MEP de rattachement** est passée.

`calendrier::mep_de` rattache un run à la dernière MEP qui le précède : un run
futur peut donc parfaitement porter une MEP passée. Sans la seconde garde, une
ligne déplacée là recevrait une date de MEP antérieure à aujourd'hui et
deviendrait **gelée sur-le-champ** — réputée appartenir à un lot déjà livré,
soustraite aux corrections ultérieures, ses fichiers réécrits. C'est
exactement le gel rétroactif que la règle prétend interdire.

Les candidats viennent de `calendrier::runs_utilisables` : un run hors fenêtre
est donc écarté en amont, sans traitement particulier.

### Une ligne déplacée ne devient pas épinglée

`plan::deplacer` met `Origine::Manuel` aujourd'hui. C'est juste pour un geste
humain isolé, et **faux pour un rapprochement de masse** : 83 lignes déplacées
seraient 83 lignes soustraites à toutes les régénérations futures. Le plan se
figerait un peu plus à chaque rapprochement, sans que rien ne le dise.

L'origine dit d'où vient l'affectation ; un rapprochement ne change pas cette
provenance, il corrige une donnée périmée. **L'origine reste donc inchangée.**

La traçabilité se porte au niveau du plan et non de la ligne : `plan_meta`
gagne une colonne `rapproche_le`. Le plan sait alors dire « j'ai été rapproché
le 28/07 », ce qui explique six mois plus tard pourquoi il ne ressemble pas à
ce que la rampe aurait produit.

## Architecture

### `rapprochement.rs` — pur, sans I/O

```rust
pub fn calculer(plan, entrees, runs, meps, aujourdhui) -> Result<Rapprochement, String>
pub fn appliquer(plan: &mut [LignePlan], r: &Rapprochement, maintenant: i64) -> Result<(), String>
```

`calculer` peut échouer : elle hérite du refus de `dedoublonner` sur un compte
dont le fichier donne deux jours de cycle contradictoires.

`appliquer` prend une **tranche**, pas un `Vec` : elle n'ajoute ni ne supprime
jamais de ligne, et le type le dit.

`appliquer` échoue si un écart désigne un compte absent du plan — incohérence
interne qui ne peut venir que d'un rapprochement calculé sur un autre plan.
Comme `ajouter` et `deplacer`, elle **vérifie tout avant d'écrire quoi que ce
soit** : un lot à moitié appliqué serait pire qu'un refus.

Module étanche, testable sans disque ni UI, comme `pid`, `modes` ou
`calendrier`.

Rejeté : étendre `plan.rs`, qui fait déjà 2 523 lignes et porte le pool, la
rampe, l'allocation, les retouches et les paramètres.

Rejeté : un mode « conservateur » de `regenerer`. Ça mettrait deux opérations
aux invariants opposés — l'une re-tire, l'autre préserve — dans la fonction la
plus centrale du module.

### L'application ne passe pas par `regenerer`

Elle mute les lignes en place, par le chemin qu'empruntent déjà `plan_retirer`
et `plan_deplacer` : `charger_pour_retouche` → `sauver_apres_retouche`. C'est
ce qui garantit littéralement zéro perturbation — aucune ré-allocation n'est
appelée.

### Deux commandes

`plan_rapprocher` charge le plan et son calendrier **persistés**
(`charger_pour_retouche` + `calendrier_du_plan`, donc les runs du plan et non
ceux affichés à l'écran), relit le fichier via `plan_entrees_from_scan`,
calcule, **n'écrit rien**.

`plan_rapprocher_appliquer` **recalcule tout depuis zéro**, applique, persiste
via `sauver_apres_retouche` — qui réécrit les fichiers MEP et supprime les
obsolètes, comme pour un retrait — et renseigne `rapproche_le`.

**Le plan se réaligne sur le nouveau fichier.** `plan_meta.fichier` et
`plan_meta.hash` deviennent ceux du fichier ouvert : le plan décrit désormais
F2, et le laisser pointer sur F1 ferait annoncer « contenu différent » par
`rapport_au_fichier` sur un plan qu'on vient précisément d'aligner. Seuls
`genere_le` et `params_yaml` restent — le plan n'a pas été régénéré.

Le diff ne transite pas par le front : ce qui s'écrit ne dépend jamais de
données remontées par le JS. En contrepartie, un fichier modifié entre les deux
clics appliquerait autre chose que ce qui a été lu — d'où l'empreinte SHA-256
rendue par `calculer`, reçue par `appliquer`, qui **refuse** si elle a bougé.

Coût : deux scans CSV complets avec jointure, un par commande. C'est la norme
du projet (`plan_lignes`, `plan_preview` font de même), et l'indication de
traitement livrée le 27/07 s'applique telle quelle.

## Refus et cas limites

### Refus francs

- pas de plan, pas de fichier ouvert, colonnes non désignées → messages
  existants (`charger_pour_retouche`, `input_path`, `plan_entrees_from_scan`) ;
- **empreinte du fichier changée entre le calcul et l'application** → refus
  explicite, « relance le rapprochement » ;
- **compte présent deux fois avec deux jours de cycle ou deux adressages
  différents** → refus, hérité de `construire_pool` (`plan.rs:104`).

Ce dernier point demande une extraction. `construire_pool` porte la règle mais
ne rend que les **éligibles** — or le rapprochement cherche précisément les
inéligibles. Le dédoublonnage est donc extrait en `plan::dedoublonner`,
partagé par les deux appelants. Refactor ciblé, **sans changement de
comportement**.

### Ce qui n'est pas une erreur

- **aucun écart** → « le plan est à jour avec le fichier ouvert », bouton
  d'application inerte ;
- **un fichier au nom différent est le cas nominal** — F2 n'a probablement pas
  le nom de F1 (extraction datée). `AutreFichier` ne bloque rien ;
- **un fichier identique n'interdit pas le rapprochement** — l'éligibilité
  vient de la base, pas du CSV : annuaire rechargé, résolution relancée,
  verdict CTC qui bascule avec le temps. Un plan peut être périmé sans qu'un
  octet du fichier ait bougé. Le bouton reste donc accessible dans les
  **quatre** états de `RapportAuFichier`.

### Avertissements, avant application

- annuaire PPF construit par cumul (`ppf_files` en compte plus d'un) → la
  perte d'éligibilité PPF n'y est pas détectable, renvoi vers « vider puis
  recharger » ;
- retraits sur MEP livrée, groupe à part → réécrit des fichiers déjà transmis ;
- **rapprochement massif** → la proportion de lignes actives retirées est
  toujours affichée ; au-delà du **quart**, elle devient un avertissement
  explicite. Un seuil chiffré plutôt qu'un jugement : « beaucoup » ne se teste
  pas ;
- répartition par plateforme modifiée par les rafraîchissements, chiffres à
  l'appui.

### Un risque qui n'existe pas sur ce chemin

`regenerer` refuse quand une MEP gelée a disparu de la configuration. Ici le
calendrier vient de `meta` : ce sont exactement les MEP qui ont produit le
plan. Le cas ne peut pas se présenter.

### Motifs de retrait

Générés, datés, lisibles dans six mois : « Rapprochement du 28/07/2026 — CTC
non prêt », « Rapprochement du 28/07/2026 — absent de comptes_2026-08.csv ».
Annulables par le bouton « Réactiver » livré le 27/07.

## Tests

TDD, test d'abord. `calculer` étant pure, presque tout se prouve sans disque
ni UI.

### `calculer`

- un compte devenu CTC non prêt, un compte absent du fichier → **deux tests
  distincts** : les motifs diffèrent, et un motif faux est ingérable plus tard ;
- **un compte inchangé ne produit aucun écart** — la promesse du chantier ;
- le jour changé part vers un run **de la même MEP** quand il en existe un ;
  sinon vers la MEP la plus proche ; sinon `Signaler` ;
- **un run déjà passé n'est jamais choisi** ;
- une ligne gelée au jour changé est signalée, **jamais déplacée** ; gelée et
  devenue inéligible, elle est proposée au retrait **et marquée** ;
- un compte à retirer n'est pas aussi déplacé — l'ordre de résolution ;
- une ligne déjà retirée est ignorée ;
- un changement de plateforme rafraîchit sans déplacer ;
- **aucun compte éligible hors plan n'est ajouté** — la garantie qu'aucun
  re-tirage ne s'est glissé là ;
- un doublon de compte avec deux jours contradictoires est refusé.

### `appliquer`

Deux tests portent tout le chantier :

- **l'origine des lignes déplacées est inchangée** — la régression la plus
  insidieuse, celle qui figerait le plan sans que rien ne le dise ;
- **les lignes inchangées le restent, champ pour champ** — l'invariant central.

Plus : le jour **et** le run sont mis à jour (sinon le déplacement ne sert à
rien) ; aucune ligne n'est ajoutée ni supprimée — un retrait marque, il ne
supprime pas.

### `plan::dedoublonner`

Extraction à comportement constant : les tests actuels de `construire_pool`
doivent rester verts **sans être touchés**. S'ils changent, l'extraction a
changé le comportement.

### Côté JS

Le faux DOM ne prouve que le câblage : bouton « Appliquer » inerte sans écart,
empreinte du fichier conservée à travers un re-rendu. Il ne dira jamais si
l'écran est lisible — **la validation GUI reste le geste de l'utilisateur**.

### Passe de mutation

Sur `rapprochement.rs`, avant de considérer le lot fini. Sur les cinq tâches
Rust du plan de charge, la mutation a trouvé un trou à chaque fois — jamais un
test manquant, toujours un test incapable d'échouer.

## Hors périmètre

- **La correction du chargement cumulatif de l'annuaire PPF** — lot séparé,
  seulement signalé ici.
- L'ajout au plan des comptes éligibles absents, nouveaux ou redevenus
  éligibles.
- Le décochage ligne à ligne avant application.
- Toute modification de `plan::regenerer` et de la rampe.

## Suites possibles

- Une maquette HTML de l'écran de revue, à valider avant tout code d'IHM.
- Le lot « chargement PPF en remplacement », qui rendrait la perte
  d'éligibilité PPF détectable et supprimerait l'avertissement prévu ici.
