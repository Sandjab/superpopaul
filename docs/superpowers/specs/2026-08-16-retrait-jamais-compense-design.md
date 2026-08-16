# Un retrait n'est jamais compensé — exclusion d'un run gelé à venir, fraîcheur de l'aperçu

Validé le 16/08/2026. Issu du premier retour d'usage GUI du chantier
« alléger un run » (v1.9.0) : trois constats, dont deux retenus comme chantier.
Ce chantier **lève une limite documentée** de la spec du 14/08 (§ 5 : « une
régénération replacera d'autres comptes pour tenir le volume des runs ») — ce
n'est plus une limite, c'est le défaut à corriger.

## Le problème

Scénario vécu : une MEP est gelée (date passée), l'utilisateur exclut 2 runs et
en allège 2 autres.

1. **Le volume retiré revient par la fenêtre.** Le moteur fait libérer aux
   retirées leur part de cible (`Preserves::consomme()` ne compte que gelées et
   épinglées) : à l'aperçu comme à un futur « Générer », la rampe re-tire des
   comptes de remplacement. Un allègement doit réduire le volume livré,
   définitivement.
2. **Un run à venir d'une MEP gelée ne peut pas être exclu.** La modale
   « Alléger… » n'offre « Exclure le run » que pour un run déjà joué ; pour un
   run à venir, prorata et sélection imposent de garder au moins un compte.
   Or les lignes d'une MEP gelée étant préservées telles quelles, l'exclusion
   a posteriori est le **seul** levier pour vider un tel run — la case
   « exclure » de la timeline n'écrit rien (paramètre de calcul).
3. **L'aperçu du paramétrage reste périmé après un geste.** La modale recharge
   le récap mais ne relance jamais le recalcul de l'aperçu : Visé / Stock /
   Placé / Reliquat décrivent le plan d'avant, jusqu'à la prochaine frappe.

## Décisions structurantes (arbitrées en brainstorming)

- **Un retrait n'est jamais compensé par un re-tirage — gelé ou non.** Règle
  uniforme, choisie contre la variante « seulement les gelés » : alléger réduit
  le volume, point. C'est l'**inversion assumée** de la décision documentée
  dans `plan.rs` (« les retirées ne comptent pas ») : commentaire et tests qui
  l'encodent sont à inverser, pas à contourner.
- **La cible saisie ne bouge pas.** Approche « les retirées consomment leur
  part » retenue contre « réécrire la cible persistée à chaque retrait »
  (mutation de la piste d'audit `params_yaml`, casse du cas cible vide = auto,
  désynchronisation à la resaisie) et contre « rafraîchir sans toucher le
  moteur » (l'aperçu montrerait le retirage refusé).
- **KPI de l'aperçu inchangés** : cible saisie affichée brute, « total » actif
  qui baisse, compteur « retirés » qui monte — la différence se lit déjà.
- **Le rafraîchissement couvre toutes les retouches**, pas seulement la modale
  « Alléger… » : le même trou de fraîcheur existe pour l'ajout, le déplacement,
  le retrait depuis le récap et la réactivation.

## 1. Moteur — les retirées consomment leur part (`plan.rs`, TDD)

- `Preserves::consomme()` compte gelées + épinglées + **retirées** : la part
  restante à tirer (`cible − consomme()`) ne change pas quand un compte est
  retiré. Ni l'aperçu ni « Générer » ne tirent de remplaçant.
- `cible_auto` : chaque préservée occupe une place — fournie par le pool si le
  compte y est encore, **ajoutée sinon**. Les retirées rejoignent gelées et
  épinglées dans le comptage `hors_pool` ; sans cela, un compte retiré disparu
  du fichier amputerait le complément d'une place.
- Invariant à verrouiller par test : **après un retrait, les comptes actifs
  restants sont exactement ceux d'avant, aucun remplaçant n'est tiré, et les
  avertissements ne changent pas.** En particulier « cible non atteinte » ne
  doit pas apparaître du seul fait d'un retrait.
- Recensement au plan d'implémentation : commentaires et tests qui encodent
  l'ancienne règle (`consomme()`, `cible_auto`, spec du 14/08 § 5, mentions
  éventuelles dans `plan_report.rs` / rapports).

## 2. Modale « Alléger… » — exclure un run à venir d'une MEP gelée

- **Condition d'offre élargie** : « Exclure le run » est proposé quand le run
  est déjà joué (comportement actuel, inchangé : mode unique) **ou** quand la
  MEP du run est passée (lignes gelées) alors que le run est à venir. Dans ce
  second cas, la bascule gagne un **troisième segment** « Exclure le run » aux
  côtés de « Retirer N — répartition conservée » et « Ne garder que ma
  sélection ».
- **Motif pré-rempli du run à venir gelé** : « Run RXX du JJ/MM/AAAA exclu — »
  (sans « a posteriori », qui ne convient qu'à un run joué). Même exigence que
  le mode actuel : le bouton reste inerte tant que rien n'est écrit après le
  tiret.
- Un run à venir d'une MEP **non** gelée ne change pas : prorata et sélection
  seulement (la régénération sait re-répartir ses comptes ; le vidage total y
  reste un non-geste).
- **Côté Rust, rien à changer** : `plan_exclure_run` n'a aucune garde de date
  (choix documenté du chantier v1.9.0). Le déblocage est purement IHM.
- L'avertissement « MEP gelée — fichier déjà transmis » existant s'applique
  tel quel.
- **Maquette HTML validée par un go explicite avant tout code UI** (règle
  projet) : modale d'un run à venir gelé, 3 segments, libellé du pré-rempli.

## 3. Câblage — l'aperçu se recalcule après chaque retouche (`app.js`)

- Après **toute** retouche réussie du plan — ajout, déplacement, retrait depuis
  le récap, réactivation, allègement, exclusion — relancer le recalcul de
  l'aperçu (le `planRecalc()` débounce existant), en plus du rechargement du
  récap déjà en place.
- Le recalcul lit les paramètres actuellement saisis à l'écran : comportement
  identique à une frappe, aucune sémantique nouvelle. Avec le moteur du § 1,
  l'aperçu recalculé montre le plan allégé sans retirage.

## 4. Limites, dites comme telles

- **La case « exclure » de la timeline n'est pas touchée** : elle reste un
  paramètre de calcul qui n'écrit rien, sans signalement de son inefficacité
  sur les lignes gelées. Piège identifié, traitement non retenu dans ce
  chantier.
- Le volume retiré n'est récupérable que par deux gestes explicites :
  réactiver les comptes, ou augmenter la cible saisie.
- La réactivation d'une ligne auto non gelée reste ce qu'elle est : la ligne
  redevient active mais n'est pas préservée à la régénération (comportement
  existant, hors périmètre).

## 5. Tests

- **Rust (TDD)** : `consomme()` avec retirées (gelées retirées, épinglées
  retirées, auto retirées) ; `cible_auto` avec retirée encore au pool / hors
  pool ; `regenerer` — invariant « actifs restants inchangés, aucun
  remplaçant, mêmes avertissements » après retrait d'une gelée puis d'une
  auto ; pas de « cible non atteinte » induit par un retrait.
- **JS (câblage, faux DOM)** : le segment « Exclure le run » offert pour un
  run à venir gelé et refusé pour un run à venir non gelé ; pré-rempli du run
  à venir gelé (sans « a posteriori ») et bouton inerte tant que la cause
  manque ; run passé inchangé (mode unique) ; recalcul de l'aperçu déclenché
  après chaque retouche réussie (les six gestes).
- **Passe de mutation** en fin de chantier, sauvegarde rafraîchie avant la
  passe (leçon v1.6.0).
