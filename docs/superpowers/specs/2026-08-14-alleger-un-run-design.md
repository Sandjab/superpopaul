# Alléger un run — aides à la saisie des retraits en lot (v1.9.0)

Validé le 14/08/2026. Suit le merge du PR #2 (« Les retraits manuels au rapport
de rapprochement », v1.8.0) : ce chantier s'appuie sur son mécanisme de
traçabilité et le complète, il ne part **qu'après** son merge.

## Le problème

Un premier run a révélé des erreurs. Conséquences côté plan : des runs suivants
ont été retirés à la main, et le volume des deux premiers runs restants a été
revu à la baisse. Il faut maintenant :

1. **Runs passés joués sans leurs comptes** : retirer d'un coup tous les CF de
   ces runs, en le traçant — le geste vécu est « exclure le run a posteriori » ;
2. **Runs à venir** : au choix, retirer N comptes en **conservant la
   distribution des plateformes** du run, ou ne garder que N comptes **choisis
   à la main**.

Aujourd'hui, chaque retrait passe par la sélection ligne à ligne du récap :
praticable pour trois comptes, pas pour un run entier ni pour un décimage
proportionnel.

## Décisions structurantes (arbitrées en brainstorming)

- **Tout aboutit au `plan::retirer` existant.** Les aides calculent des listes
  de CF ; le retrait lui-même, sa trace (`Retrait { le, motif }`) et sa
  documentation par le rapport (filigrane du PR #2) restent inchangés. Aucune
  colonne nouvelle, aucune migration — l'alternative « identifiant de geste
  stocké » est écartée pour la même raison que la colonne d'origine l'avait été
  au PR #2 (seconde source de vérité pouvant contredire les horodatages), et
  l'alternative « état “run exclu” dans la méta » créerait un second mécanisme
  de retrait à réconcilier partout.
- **Le geste vit sur la ligne du run** dans la timeline (onglet Paramétrage),
  comme l'ajout — « la décision part du run ».
- **Mode proportionnel : l'app propose, l'utilisateur valide.** La proposition
  est amendable compte par compte avant application.
- **Le rapport regroupe par geste.** Clé de regroupement : `(le, motif)` — un
  lot passé par `plan::retirer` partage déjà le même horodatage (une seule
  lecture d'horloge) et le même motif. Un geste = un chapeau + une entrée
  d'alerte, pas 150 répétitions.
- **Sémantique du « gelé » d'un retrait manuel : au moment du geste.**
  Retouche héritée de la revue du PR #2 (voir § Retouches).

## 1. Le geste — modale « Alléger… »

Une action sur la ligne du run dans la timeline, à côté de « + Ajouter ». La
modale n'offre que les modes pertinents :

- **Run passé** (`run_date < aujourd'hui`) : « Exclure le run » — toutes les
  lignes **actives** du run, épinglées comprises (exclure un run, c'est tout le
  run). Motif pré-rempli : « Run RXX du JJ/MM/AAAA exclu a posteriori — » à
  compléter (le motif reste obligatoire : le pré-remplissage ne suffit pas, il
  faut une cause).
- **Run à venir**, deux modes :
  - **Retirer N (répartition PA conservée)** : champ N ; la proposition
    s'affiche groupée par plateforme (« Esalink 4/12, Serensia 2/5… »), chaque
    compte proposé est échangeable contre un autre compte de la même
    plateforme ; validation avec motif.
  - **Ne garder que ma sélection** : liste des comptes actifs du run avec
    cases ; on coche les **gardés** ; le pied de modale affiche en permanence
    « N gardé(s) — M seront retiré(s) ». Le bouton d'application porte M.

Règles transverses : motif obligatoire (règle `plan::retirer` existante,
inchangée) ; l'avertissement « MEP gelée — fichier déjà transmis » de la modale
de retrait actuelle s'applique tel quel ; un seul appel à `plan_retirer` par
geste (c'est ce qui fait la clé de regroupement).

**Maquette HTML validée par un go explicite avant tout code UI** (règle
projet) : modale (3 modes) + rendu regroupé du rapport.

## 2. Logique pure (`plan.rs`, TDD)

Fonctions libres, sans `tauri::State`, testables sans UI :

- `cfs_actifs_du_run(plan, run_num) -> Vec<String>` — lignes non retirées du
  run, toutes origines confondues.
- `proposer_retrait_proportionnel(plan, run_num, n) -> Result<Vec<String>, String>`
  — répartit N entre les plateformes du run **aux plus forts restes**
  (le miroir des quotas de génération, proportionnels avec plancher), avec :
  - **plancher 1 inversé** : jamais le dernier compte actif d'une plateforme —
    la couverture gagnée à la génération ne se perd pas par décimage ;
  - **protections d'origine** : les lignes `Couverture` et `Manuel` (épinglées)
    ne sortent qu'en dernier recours, quand les lignes allouées ne suffisent
    pas à atteindre N ;
  - **ordre de sortie dans une plateforme** : l'inverse de la priorité
    d'allocation (`trier_par_priorite`) — sortent d'abord les comptes hors
    annuaire, puis les résolutions les plus anciennes, départage seedé ;
  - erreur si N ≥ actifs du run (c'est « Exclure le run » qui fait ça, et il
    n'est proposé que pour un run passé) ou si les planchers rendent N
    inatteignable — message disant le maximum retirable.
- « Ne garder que ma sélection » n'a pas de fonction : c'est le complément de
  la sélection dans `cfs_actifs_du_run`, calculé côté modale.

## 3. Le rapport — regroupement par geste

Dans `retraits_manuels_depuis` (commands.rs) ou en aval dans le rendu :

- Les retraits manuels sont regroupés par `(le, motif)` — horodatage **brut**
  (la seconde, pas le jour rendu) puis motif.
- **Tableau « Comptes retirés — décision manuelle »** : un groupe de taille
  > 1 est rendu sous un chapeau « N comptes retirés le JJ/MM/AAAA —
  Motif : … », ses comptes dessous ; un groupe de 1 garde le rendu actuel du
  PR #2 (une ligne CF / date / motif). Ordre : par date de geste puis motif ;
  dans un groupe, par n° de CF.
- **Alerte rouge « MEP déjà transmise »** : une entrée **par geste** contenant
  des comptes gelés (« N compte(s) retirés figuraient dans un fichier qui vous
  a déjà été transmis. Motif : … »), au lieu d'une entrée par compte. Un geste sans
  compte gelé n'y apparaît pas ; un geste partiellement gelé n'y liste que ses
  gelés (le compte N de l'entrée est celui des gelés).
- **Tuile** : inchangée — elle compte des comptes, pas des gestes.
- Un test verrouille l'invariant « un lot partage son horodatage » (même
  esprit que le verrou du filigrane du PR #2) : si `plan_retirer` se met à
  lire l'horloge par ligne, le regroupement éclate et le test le dit.

## 4. Retouches héritées de la revue du PR #2

- **Gelé-au-geste** : pour un `RetraitManuel`, `gelee` devient
  « `mep_date` déjà passée **au moment du retrait** » (comparaison de
  `retire.le` converti en date locale avec `mep_date`), au lieu d'« au moment
  du rapport ». Aligne la sémantique sur celle des écarts calculés
  (`rapprochement.rs` évalue au moment de la décision) et éteint la
  sur-alerte : un compte retiré d'une MEP alors future ne rejoint plus
  l'alerte rouge quand la MEP passe entre le geste et le rapport.
- **Bandeau** : dédoublonner le fragment « retrait(s) manuel(s)
  documenté(s) » de `compteRenduRapprochement` (une seule construction de la
  phrase, quelle que soit la branche).

## 5. Limites, dites comme telles

- Les aides retirent des comptes du **plan courant** ; elles ne touchent pas
  la rampe. Une régénération replacera d'autres comptes pour tenir le volume
  des runs — les retirés, eux, ne reviennent jamais (exclus du re-tirage,
  comportement existant).
  **Levée le 16/08/2026** : un retrait n'est plus compensé — voir
  `2026-08-16-retrait-jamais-compense-design.md`.
- Réactivations et déplacements restent non tracés au rapport (périmètre du
  PR #2 inchangé).
- Le regroupement par `(le, motif)` classe ensemble deux gestes distincts qui
  partageraient la même seconde ET le même motif — fenêtre du même ordre que
  la collision assumée du filigrane, acceptée pour les mêmes raisons
  (application de bureau mono-utilisateur).

## 6. Tests

- **Rust (TDD)** : répartition aux plus forts restes (dont arrondis), plancher
  1 par plateforme, protections `Couverture`/`Manuel`, ordre de sortie
  inversé, erreurs (N trop grand, planchers inatteignables) ;
  `cfs_actifs_du_run` (retirées exclues, origines confondues) ; regroupement
  du rapport (chapeau à partir de 2, groupe de 1 inchangé, alerte par geste,
  geste partiellement gelé) ; gelé-au-geste (retrait avant MEP, rapport
  après). Verrou : un lot partage son horodatage.
- **JS (câblage)** : modes offerts selon la date du run, compteur
  « gardés/retirés », motif obligatoire (bouton inerte), un seul
  `plan_retirer` par geste.
- **Passe de mutation** en fin de chantier ; exclure le test instable connu
  `rafale_5xx_ouvre_le_breaker_une_seule_fois_puis_reprend` (constat du
  PR #2) tant qu'il n'est pas stabilisé.
