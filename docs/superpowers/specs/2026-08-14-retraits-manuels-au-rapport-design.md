# Les retraits manuels au rapport de rapprochement — design

Un plan de charge est établi, puis des comptes doivent en être **exclus par
décision** — pas parce que le fichier a changé, pas parce que l'éligibilité est
tombée : parce qu'on ne veut plus les facturer. Le geste existe et il est le
bon : sélection au récap, « Retirer… », motif obligatoire (`plan::retirer`,
`plan.rs:927`).

Le trou est ailleurs. Ces retraits **réécrivent les fichiers transmis** et
n'apparaissent dans **aucune pièce transmise**.

## Ce que l'application produit aujourd'hui

Le retrait manuel laisse trois traces, toutes internes ou muettes :

- la ligne conserve son motif et sa date (`Retrait { le, motif }`,
  `plan.rs:520`), lisibles **dans l'application** au récap, filtre « retiré » ;
- le classeur du périmètre marque le compte `Retire` (`plan_xlsx.rs:57`) — il
  dit l'état, jamais la décision ni sa date ;
- le rapport de plan compte les retirés dans un compteur (`plan_report.rs:365`)
  et les exclut de tous les autres chiffres.

Aucune de ces traces n'est un **delta**. Le destinataire des `.txt` cumulatifs
voit des comptes disparaître d'un lot au suivant sans qu'aucun document ne le
dise.

## Ce qui manque, et pourquoi le rapport ne le comble pas

Le rapport de rapprochement est la seule pièce transmise qui décrit un delta
(`rapprochement_report.rs`, livré le 28/07). Il est **structurellement aveugle**
aux retraits manuels, en deux endroits qui se renforcent :

1. `rapprochement::calculer` saute toute ligne déjà retirée (`rapprochement.rs:94`).
   Un compte retiré à la main avant le rapprochement ne produit aucun écart —
   le test `une_ligne_deja_retiree_est_ignoree` (`rapprochement.rs:517`) verrouille
   même le fait qu'il n'est pas non plus compté parmi les inchangées ;
2. seul `plan_rapprocher_appliquer` écrit un rapport. Les retouches manuelles
   n'en produisent pas — décision du 28/07, motivée : « elles ne portent pas de
   delta calculé ».

Conséquence, dans le scénario qui a fait remonter le besoin : on retire des
comptes à la main, on relance une résolution complète, on rapproche. Les trois
gestes réécrivent les mêmes fichiers, et le document qui les accompagne ne
mentionne que le troisième.

## Périmètre

Ce lot rend les **retraits manuels** visibles dans le rapport de rapprochement.
Il ne touche ni au calcul du rapprochement, ni au geste de retrait.

| Modification manuelle | Traitée ici | Détectable |
|---|---|---|
| Retrait (`plan_retirer`) | **oui** | date + motif portés par la ligne |
| Réactivation (`plan_annuler_retrait`) | non | **non** — la trace est effacée |
| Ajout (`plan_ajouter`) | non | oui (`origine` + `planned_at`) |
| Déplacement (`plan_deplacer`) | non | **non** — aucun horodatage |

Le tableau commande le vocabulaire du document : il parle de **comptes retirés
à la main**, jamais de « modifications manuelles ». Une section qui promettrait
les secondes serait silencieusement incomplète sur deux lignes du tableau — et
un document transmis qui sous-entend l'exhaustivité est pire qu'un document qui
se tait.

## Reconnaître un retrait manuel non encore rapporté

Rien ne marque l'origine d'un retrait : `Retrait` porte une date et un motif,
pas sa provenance. Elle se **déduit** de deux horodatages déjà en base.

`plan_rapprocher_appliquer` calcule `maintenant` **une seule fois**
(`commands.rs:1919`) et s'en sert pour deux écritures : les retraits que le
rapprochement pose (`appliquer`, `rapprochement.rs:241`) et `meta.rapproche_le`
(`commands.rs:1929`). Les retraits d'un rapprochement portent donc
**exactement** la date de rapprochement enregistrée.

D'où la règle :

> Un retrait est manuel et non encore rapporté quand
> `retire.le > meta.rapproche_le`.
> Si `rapproche_le` est `None`, **tous** les retraits présents le sont : seul
> `plan::retirer` a pu les poser.

La comparaison est **stricte**, et c'est ce qui exclut les retraits du dernier
rapprochement — à la seconde près, pas par convention.

**Cette coïncidence d'horodatage est le seul point fragile du lot**, et elle
doit être verrouillée par un test (voir plus bas) : un `Utc::now()` calculé deux
fois au lieu d'une, dans le sens défavorable, ferait réapparaître les retraits du
rapprochement *n* dans le rapport *n+1*, sous la mauvaise étiquette. Le
compilateur ne voit rien de tel.

Rejeté : **une colonne d'origine sur `Retrait`** (`Manuel` / `Rapprochement`).
Elle demande une migration `ALTER TABLE` sur `plan_cf`, un variant à poser dans
`rapprochement::appliquer`, et introduit une **seconde source de vérité** qui
peut contredire les horodatages sans que rien ne le signale. L'invariant tient
en une assertion ; la colonne tiendrait en une migration plus un champ à ne
jamais oublier.

### La capture se fait avant l'écrasement

Comme `fichier_avant` (`commands.rs:1901`), la liste est établie **avant**
l'étape 5 du flux, qui écrase `meta.rapproche_le`. Elle se lit sur `lignes`
avant `appliquer` : `appliquer` ne touche jamais une ligne déjà retirée — les
écarts ne peuvent pas en désigner, `calculer` les ayant sautées — mais capturer
au même endroit que les deux autres photos évite d'avoir à redémontrer cette
étanchéité à chaque relecture.

## Modèle

`rapprochement_report.rs` reste **pur** : ni horloge, ni disque. Les dates
arrivent formatées.

```rust
/// Un compte retiré à la main, tel que le rapport en parle.
pub struct RetraitManuel<'a> {
    pub cf: &'a str,
    /// Date du retrait, déjà formatée — ce module n'a pas d'horloge.
    pub le: &'a str,
    /// Saisi par l'utilisateur. **Texte libre**, échappé au point d'insertion.
    pub motif: &'a str,
    /// La MEP de la ligne est passée : le compte figure dans un fichier déjà
    /// transmis. Même conséquence qu'un `Ecart.gelee`, même traitement.
    pub gelee: bool,
}

/// Ce que la liste des retraits manuels prend pour origine. Le document écrit
/// la date ET ce qu'elle désigne : « depuis le 28/07 » ne dit pas au lecteur
/// pourquoi la liste commence là.
pub enum Depuis<'a> {
    DernierRapprochement(&'a str),
    /// Le plan n'a jamais été rapproché.
    GenerationDuPlan(&'a str),
}
```

`RapprochementReportData` gagne deux champs :

```rust
    /// Triés par date de retrait puis n° de CF.
    pub retraits_manuels: &'a [RetraitManuel<'a>],
    pub depuis: Depuis<'a>,
```

`RapprochementVue` (`commands.rs:1803`) gagne un **compte**, pas une liste — le
lot 2 en a besoin pour décider si son bouton est actif, et l'écran n'affiche pas
le détail (voir « Ce que l'écran de revue ne montre pas ») :

```rust
    /// Retraits manuels que ce rapprochement documentera.
    pub retraits_manuels: usize,
```

## Contenu du document

### Une cinquième tuile, rendue seulement si elle est non nulle

Le résumé chiffré porte aujourd'hui quatre tuiles — retirés, déplacés,
plateformes corrigées, inchangées — qui vérifient une **identité** avec les
signalements, seuls à n'avoir pas de tuile :
`inchangées + retirés + déplacés + rafraîchis + signalés = actives`, où
`actives = inchangees + ecarts.len()` (`rapprochement_report.rs:153`). Les
retraits manuels sont **hors de cette arithmétique** — `calculer` les a sautés,
ils ne sont ni dans `ecarts` ni dans `inchangees`.

Les fondre dans « comptes retirés » ferait donc dépasser le total dans un
document transmis, et mélangerait deux provenances sous un même chiffre. Ils
prennent une tuile à eux, dont la sous-ligne dit qu'elle est hors du compte :
« retirés à la main · hors du calcul ».

Elle **n'est pas rendue à zéro**, contrairement aux quatre autres. Celles-là
décrivent ce que le rapprochement a examiné, et « 0 déplacé » est un constat ;
une tuile à zéro sur des retraits manuels parlerait d'un geste qui n'a pas eu
lieu. C'est la règle déjà appliquée aux tableaux (« une nature sans écart ne
produit pas de tableau vide »).

### Une troisième section de retraits, groupée avec les deux autres

Le document range ses tableaux par **conséquence pour le destinataire**, pas par
provenance. Les trois questions « quels comptes ne sont plus dans mes
fichiers ? » se lisent ensemble :

1. Comptes retirés — éligibilité perdue *(inchangé)*
2. Comptes retirés — disparus du fichier *(inchangé)*
3. **Comptes retirés — décision manuelle** *(nouveau)*
4. Comptes déplacés — jour de cycle changé *(renuméroté)*
5. Plateformes corrigées *(renuméroté)*

Colonnes : **N° de CF · Retiré le · Motif**.

La date est ce qui distingue cette section des deux précédentes : leurs comptes
sont retirés *maintenant*, par le rapprochement en cours ; celui-ci l'a été à un
moment quelconque depuis la dernière note, éventuellement des semaines plus tôt.
Sans la date, le lecteur croit à une décision du jour.

Pas de colonne MEP, pour rester aligné sur les deux tableaux voisins. Le seul
cas où la MEP compte vraiment — le compte figure dans un fichier déjà transmis —
a sa propre section, en rouge et en tête.

Sous-titre, avec la date de référence et sa nature :

> Retirés à la main depuis le *<date>* — *dernier rapprochement* | *génération
> du plan*. Ces comptes ne figurent dans aucun fichier de ce lot.

### Les retraits manuels gelés rejoignent la section rouge

« Retrait portant sur une mise en production déjà transmise »
(`rapprochement_report.rs:232`) existe pour une conséquence, pas pour une
provenance : les `.txt` étant cumulatifs, le destinataire tient une version
antérieure du fichier où le compte figurait. Un retrait manuel sur une MEP
passée produit **exactement** cette situation.

La formulation en place fonctionne telle quelle — elle cite le motif, et pour un
retrait manuel le motif est la phrase de l'utilisateur, qui dit la décision
mieux que n'importe quel libellé généré. Les entrées manuelles sont **ajoutées à
la suite** des calculées, dans un ordre déterministe.

Un retrait manuel gelé figure donc **aux deux endroits** : section rouge et
tableau ③. C'est déjà le sort des retraits calculés gelés — la section rouge met
en évidence, elle ne dispense pas du tableau.

### Ordre de la liste

Par **date de retrait croissante, puis n° de CF**. Une liste de décisions se lit
comme un journal ; et un retrait en lot pose la même seconde sur toutes ses
lignes, que le n° de CF départage.

## Lot 2 — appliquer quand il n'y a que des retraits à documenter

Livrable **indépendant** du lot 1, sur le modèle du 27/07.

Aujourd'hui, un rapprochement sans écart affiche « ✓ Le plan est à jour avec le
fichier ouvert », bouton inerte (`app.js:2746`), et n'écrit rien. Or c'est un cas
réel du scénario visé : on retire des comptes à la main, on relance une
résolution qui ne change aucun verdict, on rapproche — zéro écart, et le lot
part **sans note** alors qu'il y a quelque chose à dire.

Rien n'est perdu pour autant : `rapproche_le` n'avance qu'à l'application, donc
les retraits non rapportés le seront au prochain rapprochement appliqué. Ce qui
manque, c'est la note **de ce lot-ci**.

La condition change donc de nature : **le rapport est produit quand il y a
quelque chose à dire**, pas quand il y a un écart.

- `ecarts` vide **et** `retraits_manuels` à zéro → écran et bouton inertes,
  inchangé ;
- `ecarts` vide **et** `retraits_manuels` non nul → bouton actif, libellé
  **« Produire la note de livraison »**, et un texte qui dit ce que ça fait :
  aucun compte ne bouge, le rapport est écrit, le plan se réaligne sur le
  fichier ouvert.

Le libellé s'adapte à ce sur quoi il agit — c'est le motif déjà retenu pour
« Réactiver *n* retiré(s)… » (27/07), plutôt qu'un second bouton permanent qui
serait inerte les neuf dixièmes du temps.

**Aucun changement au chemin d'application.** `plan_rapprocher_appliquer`
traverse déjà le cas sans écart sans broncher : `appliquer` sur une liste vide
ne mute rien, le réalignement de `meta`, l'écriture des fichiers et celle du
rapport se font comme d'habitude. Seuls la condition et le libellé côté JS
changent, plus le compte remonté par `plan_rapprocher`.

### Ce que l'écran de revue ne montre pas

Le **détail** des retraits manuels n'est pas ajouté à la revue : celui qui
applique vient de les faire, et la revue sert à valider ce qui va être décidé,
non à relire ce qui l'a déjà été. Seul le **nombre** apparaît, parce qu'il change
ce que le bouton propose.

Limite assumée : sur un plan repris après plusieurs semaines, ce nombre peut
surprendre. Le récap et son filtre « retiré » répondent déjà à « lesquels ».

## Refus et cas limites

- **`rapproche_le` absent** → la référence est `genere_le`, et le document écrit
  « depuis la génération du plan ». Jamais de date nue sans ce qu'elle désigne.
- **Retrait manuel posé dans la même seconde qu'une application de
  rapprochement** → classé avec ceux du rapprochement, donc jamais listé.
  Fenêtre d'une seconde sur une application de bureau mono-utilisateur :
  **accepté**, pas contourné — le contourner demande la colonne d'origine
  rejetée plus haut.
- **Réactivation manuelle** → `annuler_retrait` remet `retire` à `None`
  (`plan.rs:964`) : la trace disparaît. Un compte réactivé depuis la dernière
  note **réapparaît** dans les fichiers sans qu'aucun document ne le dise. La
  limite est énoncée ici pour qu'elle ne se découvre pas à l'usage ; la lever
  demande un journal par ligne, hors périmètre.
- **Aucun retrait manuel** → ni tuile, ni section, ni ligne rouge.
- **Motif contenant du balisage** → échappé au point d'insertion, comme le
  reste. Nouveauté à ne pas manquer : jusqu'ici les motifs du rapport étaient
  **générés** (`format!("{stamp} — {apres}")`) ; celui-là est **saisi**. C'est la
  première chaîne d'origine humaine du document.

## Tests

TDD, test d'abord.

### `rapprochement_report::render`

Avec le helper `corps()` en place — une chaîne cherchée dans le document entier
matche la feuille de style.

- la section **n'existe pas** quand la liste est vide, et la tuile non plus ;
- un retrait manuel gelé apparaît **dans les deux** endroits : section rouge et
  tableau ③ — la mise en évidence ne dispense pas du tableau ;
- la date de référence est écrite **et qualifiée** : `DernierRapprochement` et
  `GenerationDuPlan` ne produisent pas la même phrase ;
- l'ordre rendu est celui annoncé (date puis CF), sur un jeu où les deux clés
  départagent ;
- **échappement du motif** : `<script>` et `&` dans un motif saisi ressortent
  échappés, une seule fois ;
- les retraits manuels ne sont comptés **ni** dans « comptes retirés » **ni**
  dans « lignes inchangées » — l'identité arithmétique rappelée plus haut doit
  tenir inchangée avec une cinquième tuile non nulle à côté.

### Côté commande

- **après une application, la liste des retraits manuels non rapportés est
  vide.** C'est le test qui verrouille le filigrane ; sans lui, la coïncidence
  d'horodatage peut se défaire en silence ;
- un retrait fait entre deux rapprochements est listé **exactement une fois** :
  présent au rapport *n+1*, absent au *n+2* ;
- la liste est bien calculée sur le `rapproche_le` **d'avant** l'écrasement — un
  test qui échouerait si la capture glissait après l'étape 5 ;
- `rapproche_le` à `None` → référence sur `genere_le`.

### Côté JS (lot 2)

Le faux DOM ne prouve que le câblage.

- zéro écart et zéro retrait manuel → bouton inerte ;
- zéro écart et *n* retraits → bouton actif, libellé « Produire la note de
  livraison » ;
- les doublures de `plan_rapprocher` remontent le nouveau champ — le compilateur
  Rust ne les couvre pas.

### Passe de mutation

Sur les fonctions ajoutées à `rapprochement_report.rs` et sur le filtre des
retraits manuels. Chaque tâche Rust de ce projet en a sorti au moins un test
incapable d'échouer.

## Maquettes

Convention du projet : **go explicite avant tout code d'IHM, libellé compris**.
Deux surfaces, deux fichiers, produits le 14/08 :

- `docs/superpowers/maquettes/2026-08-14-rapport-retraits-manuels.html` — le
  rapport avec la cinquième tuile, le tableau ③ et l'entrée manuelle dans la
  section rouge ;
- `docs/superpowers/maquettes/2026-08-14-note-de-livraison.html` — la modale
  « sans écart » du lot 2, son libellé de bouton et son texte d'explication.

### La maquette du 28/07 a divergé du code livré

Elle n'a **pas** été reprise comme base, et ne doit pas servir de référence :
ses tableaux de retrait portent des colonnes « Run quitté » et « MEP » que
`rapprochement_report.rs` n'a jamais rendues — le code livré y écrit « Motif ».
La maquette du 14/08 est construite sur le **rendu réel**, sans quoi la
validation porterait sur un document qui n'existe pas.

Constat noté, non traité : réaligner la maquette du 28/07 sur ce qui ship, ou
ajouter les colonnes au code, est une question que ce lot ne pose pas.

## Hors périmètre

- **Ajouts et déplacements manuels.** L'ajout serait détectable (`Origine::Manuel`
  et `planned_at > rapproche_le`, `plan.rs:887`), le déplacement ne l'est pas —
  `deplacer` marque l'origine sans horodater (`plan.rs:916`). Livrer les ajouts
  seuls donnerait une section « modifications manuelles » incomplète sans le
  dire ; le tableau du périmètre existe pour rendre ce choix explicite.
- **Réactivations** — trace effacée par conception.
- **Rapport propre à `plan_retirer`.** Un second gabarit, un second nommage, et
  un document par geste là où le lot se transmet en une fois. Le geste qui
  transmet est le rapprochement : c'est lui qui porte la note. Reste vrai qu'un
  plan qu'on ne rapproche jamais ne produit aucune note — le lot 2 traite le cas
  « rien à rapprocher », pas le cas « on ne rapproche pas du tout ».
- **Journal cumulatif en base** — déjà écarté au 28/07, et c'est ce qu'il
  faudrait pour les réactivations et les déplacements.
- **Harmonisation des colonnes des trois tableaux de retrait** — toucher un
  document validé pour une question que ce lot ne pose pas.

## Suites possibles

- Un **journal par ligne** (`plan_historique`), qui rendrait visibles
  réactivations et déplacements, et répondrait à « qu'est-il arrivé à ce compte
  depuis mars ». C'est un autre produit : une consultation, pas une pièce
  transmise.
