# Rapport de rapprochement — design

Appliquer un rapprochement retire des comptes d'un périmètre de facturation,
en déplace d'autres d'une mise en production à une autre, et réécrit les
fichiers transmis à celui qui exécute la facturation. Aujourd'hui, rien de tout
cela ne laisse de trace lisible.

Le besoin est de produire, **à chaque application**, une pièce qui accompagne
les livrables et dit au destinataire ce qui change de son côté.

## Ce que l'application produit aujourd'hui

`plan_rapprocher_appliquer` (`commands.rs:1730`) délègue l'écriture à
`sauver_apres_retouche` (`commands.rs:1486`), partagé avec `plan_ajouter`,
`plan_deplacer`, `plan_retirer` et `plan_annuler_retrait`. Trois livrables en
sortent, solidaires par conception — « laisser un livrable en arrière le ferait
diverger de la base en silence » :

- le **plan en base** (`ecrire_plan`), plus `plan_meta.rapproche_le`, `fichier`
  et `hash` réalignés sur le fichier rapproché ;
- les **listes de comptes par MEP**, `<souche>_plan_mep_<id>_<date>.txt`
  (`commands.rs:1169`) : un n° de CF par ligne, trié, dédoublonné, et
  **cumulatif** — le fichier de la MEP *n* contient les comptes des MEP 1 à *n* ;
- le **classeur du périmètre**, `<souche>_plan_comptes.xlsx`, chemin unique
  décidé en un seul endroit (`chemin_classeur`, `commands.rs:684`).

Les fichiers de MEP d'une génération précédente devenus vides sont supprimés,
et leurs chemins remontent jusqu'au bandeau de l'écran.

## Ce qui manque

Le `Rapprochement` calculé est affiché à l'écran de revue, puis **perdu**. Il ne
transite même pas par le front pour l'application : `plan_rapprocher_appliquer`
le recalcule côté Rust, pour que ce qui s'écrit ne dépende jamais de données
remontées par le JS. Après le clic, il ne reste qu'une phrase de bandeau et un
horodatage en base.

Conséquences :

- « pourquoi ce compte n'est plus dans la MEP 3 ? » n'a aucune réponse
  consultable, alors que la question se pose des mois plus tard ;
- l'écran applique **en bloc** : on ne peut pas reconstituer le raisonnement
  compte par compte après coup ;
- le destinataire des fichiers reçoit des `.txt` cumulatifs modifiés sans rien
  qui explique ce qui a bougé.

Le rapport de plan (`plan_rapport`, `commands.rs:1766`) ne comble pas ce trou :
il décrit **l'état** du plan, se régénère à la demande, et n'a jamais connu le
delta.

## Finalité retenue

Le rapport est une **pièce transmise avec les livrables**, destinée à celui qui
exécute la facturation. Ce choix commande le reste : contenu autoportant,
résumé en tête, formulations qui se suffisent sans l'application sous les yeux,
et **un fichier conservé par rapprochement** — contrairement au classeur et au
rapport de plan, qui ont un chemin unique réécrit.

## Le geste

Aucun. Cliquer sur « Appliquer » écrit le rapport en même temps que les `.txt`
et le classeur. Son chemin apparaît dans le bandeau de compte rendu, à côté des
fichiers obsolètes supprimés.

Le rapport n'est produit que par ce chemin. Les retouches manuelles
(`plan_ajouter`, `plan_deplacer`, `plan_retirer`, `plan_annuler_retrait`) n'en
produisent pas : elles ne portent pas de delta calculé.

Un rapprochement sans écart ne produit pas de rapport — l'écran de revue
n'offre alors pas d'appliquer (`app.js:2550`), le cas ne se présente pas.

## Nommage

`<souche>_rapprochement_AAAA-MM-JJ_HHMMSS.html`, dans le répertoire de sortie
résolu comme pour les autres livrables (`resolved_out_dir`).

Horodatage **à la seconde** : le document est transmis, et deux rapprochements
rapprochés dans le temps ne doivent pas s'écraser silencieusement. Une perte de
ce genre ne se remarque jamais.

Le préfixe évite `_plan_mep_` : `fichiers_obsoletes` (`commands.rs:1118`)
sélectionne exactement `<souche>_plan_mep_*.txt` pour suppression, et un rapport
qui tomberait dans ce filtre serait balayé au rapprochement suivant. Il évite
aussi `_plan_comptes` et `_plan.html`, qui sont des chemins uniques réécrits.

## Architecture

### `rapprochement_report.rs` — pur, sans I/O

Nouveau module calqué sur `plan_report.rs` : aucune I/O, aucune horloge, aucune
dépendance à Tauri. Il expose une struct de données et
`render(&RapprochementReportData) -> String`, et réutilise
`report::{esc, fmt_int, CSS}`.

```rust
pub struct RapprochementReportData<'a> {
    /// Nom du fichier qui a produit le plan, capturé AVANT réalignement.
    pub fichier_avant: &'a str,
    pub fichier_apres: &'a str,
    /// SHA-256 du fichier rapproché : le destinataire le compare.
    pub empreinte: &'a str,
    /// Déjà formatée par `report::date_fr_longue` — le module n'a pas d'horloge.
    pub date_longue: &'a str,
    pub version: &'a str,
    pub rapprochement: &'a Rapprochement,
    /// Fichiers de MEP réellement écrits. Vue propre au module (`FichierLivre`)
    /// et non `commands::FichierMep` : le rapport nomme les fichiers, il ne
    /// connaît pas de chemins, et le module pur n'a pas à dépendre de
    /// `commands`. La commande fait la conversion, qu'elle devait faire de
    /// toute façon pour extraire le nom du chemin.
    pub fichiers: &'a [FichierLivre<'a>],
    /// Fichiers de MEP supprimés parce que leur MEP s'est vidée.
    pub obsoletes: &'a [String],
    /// Avertissement d'annuaire PPF incomplet, s'il y a lieu.
    pub annuaire_incomplet: Option<&'a str>,
}

/// Un fichier de livraison, tel que le rapport en parle : par son nom.
pub struct FichierLivre<'a> {
    pub nom: &'a str,
    pub mep_id: usize,
    pub mep_date: &'a str,
    pub comptes: usize,
}

/// Où se trouvait une ligne avant le rapprochement.
pub struct PositionAvant {
    pub run_num: String,
    /// ISO, comme partout ailleurs dans le projet.
    pub run_date: String,
    pub mep_id: usize,
}
```

`RapprochementReportData` porte aussi :

```rust
    /// Position d'origine des lignes, capturée AVANT `appliquer`. Clé : n° de CF.
    pub origines: &'a BTreeMap<String, PositionAvant>,
```

**Pourquoi cette capture.** `Action::Deplacer` ne porte que la **destination**
(`run_num`, `run_date`, `mep_id`, `mep_date`), et `appliquer` mute les lignes en
place : après application, le run d'origine n'existe plus nulle part. Or le
rapport l'affiche — le destinataire cherche pourquoi *son* fichier de MEP 3 a
changé, et « le compte est au run 13 » ne le lui dit pas, quand « le compte
passe du run 12 au run 13 » le lui dit.

La commande construit donc la table depuis `lignes` **avant** `appliquer`.

### `sauver_apres_retouche` remonte ce qu'il a écrit

Son type de retour passe de `Vec<String>` à `(Vec<FichierMep>, Vec<String>)`.
L'information existe déjà : `ecrire_fichiers_mep` la produit et elle est jetée
(`commands.rs:1494`). Les quatre autres appelants ignorent le premier champ.

`FichierMep` (`commands.rs:1103`) gagne un champ `mep_date: String` — elle porte
aujourd'hui `chemin`, `mep_id` et `comptes`, mais pas la date, que le rapport
affiche. La valeur est déjà sous la main au moment de l'écriture : la boucle
itère sur des couples `(mep_id, mep_date)` en ISO (`commands.rs:1151`). Il s'agit
d'un **ajout** de champ à une struct `Serialize` — le JS existant n'en lit aucun,
rien ne casse. L'alternative, reparser la date depuis le nom du fichier, ferait
du nom un format à maintenir en plus du chemin.

Le helper reste **ignorant du rapprochement** : il dit ce qu'il a fait,
l'appelant décide quoi en faire. L'alternative — lui passer un
`Option<&Rapprochement>` et le laisser écrire le rapport — a été écartée : elle
apprend à un helper partagé une notion que quatre de ses cinq appelants
ignorent, et rend le prochain livrable plus dur à ajouter. Refaire la séquence
dans la commande a été écarté aussi : elle doit rester solidaire.

### Flux dans `plan_rapprocher_appliquer`

1. `calculer_rapprochement` → écarts, empreinte courante, lignes, meta,
   avertissement d'annuaire ;
2. garde d'empreinte (inchangée) ;
3. **capture de `meta.fichier`** — l'étape 5 l'écrase, et le rapport doit
   nommer le fichier d'origine ;
4. `rapprochement::appliquer` — tout ou rien : si elle échoue, le `?` sort avant
   toute écriture ;
5. réalignement de `meta` sur le fichier rapproché (inchangé) ;
6. `sauver_apres_retouche` → fichiers écrits + obsolètes supprimés ;
7. `render`, puis écriture du rapport ;
8. retour `{ obsoletes, rapport }`.

L'ordre importe : le rapport annonce les nombres de comptes **des fichiers
réellement produits** à l'étape 6. L'écrire avant obligerait à les recalculer,
avec le risque classique de deux calculs qui divergent.

### `annuaire_incomplet` cesse d'être ignoré

`plan_rapprocher_appliquer` le jette aujourd'hui (`_annuaire_incomplet`), avec
un commentaire disant que l'application n'a rien à en dire. Vrai pour l'écran,
faux pour le rapport : sans cet avertissement, le document affirme une
exhaustivité qu'il n'a pas. Le commentaire est amendé en conséquence.

### Retour de commande

`Vec<String>` devient une struct sérialisée :

```rust
#[derive(Serialize)]
pub struct RapprochementApplique {
    pub obsoletes: Vec<String>,
    pub rapport: String,
}
```

⚠️ **Changement de forme d'un retour de commande.** Trois doublures JS rendent
`[]` pour `plan_rapprocher_appliquer` (`tests/rapprochement.test.js:57`, avec
les assertions lignes 69 et 93). Le compilateur Rust ne les couvre pas : elles
doivent être migrées explicitement, sans quoi elles mentiraient au code.

## Contenu du document

Dans l'ordre de lecture, pour quelqu'un qui reçoit les fichiers sans avoir
touché à l'application.

1. **En-tête** — « Rapprochement du plan de charge », `<fichier d'origine>` →
   `<fichier rapproché>`, date longue en français, empreinte SHA-256, version.
2. **Résumé chiffré** — comptes retirés (dont non éligibles / disparus),
   déplacés, plateformes corrigées, lignes inchangées. Même vocabulaire que le
   bandeau de l'application, pour qu'un échange oral ne se perde pas en
   traduction.
3. **Avertissements du calcul** (`rapprochement.avertissements`), puis
   **séparément** l'avertissement d'annuaire PPF incomplet. Cette séparation
   reprend une décision verrouillée du chantier précédent : les premiers
   décrivent ce que le rapprochement *fait*, le second prévient qu'il est
   *incomplet*. Dans un document transmis, l'enjeu monte d'un cran — sans lui,
   « 0 éligibilité perdue » se lit comme un constat.
4. **Retraits portant sur une MEP déjà transmise** — section distincte, en
   évidence, avant les autres écarts. Seul cas où le rapport annonce qu'un
   compte présent dans un fichier **déjà transmis** n'y sera plus : les `.txt`
   étant cumulatifs, le destinataire a une version antérieure entre les mains.
   Le champ `Ecart.gelee` le porte.
5. **Les écarts par nature**, dans l'ordre du calcul (retrait > déplacement >
   rafraîchissement) : éligibilité perdue, disparus du fichier, déplacés,
   plateforme corrigée.
6. **À traiter à la main** — les `Action::Signaler` : la ligne gelée dont le
   jour change (`rapprochement.rs:140`) et le jour illisible. Ils se lisent
   « reste à trancher », jamais « fait ».
7. **Fichiers de livraison produits** — nom du `.txt`, n° et date de MEP,
   nombre de comptes ; puis les supprimés, avec leur raison.
8. **Pied** — renvoi au classeur pour le détail compte par compte, que le
   rapport ne duplique pas.

Ne figurent **pas** dans le rapport : le calendrier des runs et la répartition
par plateforme. Le rapport de plan les porte déjà, et les redire créerait deux
documents qui peuvent diverger.

### Règles de rendu

- Le **jour illisible** (`apres: 0`, sentinelle hors du domaine 1–31) se rend en
  toutes lettres. Le document ne contient jamais « jour 0 ».
- Quand le jour change mais que le run cible est le même — cas réel, le calcul
  produit un écart dès que le jour lu diffère (`rapprochement.rs:122`) — le run
  n'est pas répété : une seule valeur, suivie de « même run » en gris. La
  comparaison se fait entre `origines[cf].run_num` et la destination portée par
  l'action.
- Le **rouge est réservé** aux retraits sur MEP déjà transmise. Un seul usage,
  sinon il ne veut plus rien dire.

### Maquette

`docs/superpowers/maquettes/2026-07-28-rapport-rapprochement.html`, validée.
Le CSS est repris de `report.rs::CSS` sans modification ; les ajouts sont
délimités en fin de feuille sur le modèle du chantier « rapport de plan » :
`.warn.danger`, `.chg`/`.old`/`.arr`, `.same`, `.hash`, `.todo`, `tr.gone`.

## Refus et cas limites

### Échec d'écriture du rapport

L'erreur remonte, l'application est conservée. C'est déjà le sort du classeur :
le plan est en base avant lui, et un `?` qui échoue laisse un état partiel
signalé plutôt que silencieux. Aucun rollback n'existe sur ce chemin, et en
inventer un ici serait une exception à une règle établie.

Le message doit dire **les deux choses** : « Le rapprochement a été appliqué,
mais le rapport n'a pas pu être écrit : … ». C'est le seul endroit du flux où la
formulation change ce que l'utilisateur va faire ensuite : un message nu ferait
croire à un échec total et pousserait à relancer, or relancer recalculerait un
rapprochement désormais **sans écart**, et le document serait perdu.

### Ce qui n'est pas une erreur

- Aucun fichier obsolète supprimé : la section correspondante ne s'affiche pas.
- Aucun écart d'une nature donnée : pas de tableau vide.
- Aucun `Signaler` : pas de section « à traiter à la main ».

## Tests

TDD, test d'abord, conformément au projet.

### `rapprochement_report::render`

Avec un helper `corps()` qui isole le corps du document, comme
`plan_report::tests` (`plan_report.rs:418`) : une chaîne cherchée dans un
document riche a presque toujours plus d'un producteur, et un test qui trouve sa
chaîne dans la feuille de style ne peut plus échouer.

- un retrait sur MEP déjà transmise apparaît dans **sa** section, pas seulement
  dans le tableau des retraits — c'est la mise en évidence qui est vérifiée ;
- `apres: 0` se rend en toutes lettres ; le document ne contient jamais
  « jour 0 » ;
- un déplacement vers le même run n'écrit pas deux fois la valeur ;
- l'avertissement d'annuaire est rendu quand il est fourni, absent sinon ;
- les `.txt` produits sont listés avec leur nombre de comptes ; les supprimés
  apparaissent ;
- **échappement** : un n° de CF ou un motif contenant `<script>` ou `&` sort
  échappé — le CSV est une entrée non fiable ;
- un `Signaler` n'est jamais compté parmi les changements appliqués ;
- une nature sans écart ne produit pas de tableau vide.

### Côté commandes

- le rapport suit le répertoire de sortie configuré ;
- son nom n'est **pas** sélectionné par `fichiers_obsoletes` — non-régression
  directe sur le risque qu'il se fasse balayer par le ménage ;
- `ecrire_fichiers_mep` remonte pour chaque fichier la date de MEP qui figure
  dans son nom : les deux viennent de la même source, un test le verrouille.

### Côté JS

- les trois doublures de `plan_rapprocher_appliquer` migrent vers
  `{obsoletes, rapport}` ;
- le bandeau affiche le chemin du rapport.

### Passe de mutation

Sur le nouveau module. Chacune des tâches précédentes de ce projet en a sorti au
moins un test incapable d'échouer — jamais un test manquant, toujours un test
qui ne prouvait rien.

## Hors périmètre

- **Aperçu avant application.** Produire un rapport « à blanc » depuis l'écran
  de revue pour faire valider le changement avant de toucher au plan :
  deux chemins de production et deux états à distinguer dans le document, pour
  un besoin non exprimé.
- **Journal cumulatif en base.** L'autre forme envisagée — consulter l'histoire
  d'un compte depuis l'écran du plan. Répond mieux à « qu'est-il arrivé à ce
  compte depuis mars », mais c'est un autre produit, pas une pièce transmise.
- **Chargement PPF en remplacement.** Lot séparé déjà décidé. Tant qu'il n'est
  pas fait, le rapport porte l'avertissement d'annuaire incomplet.
- **Rapport pour les retouches manuelles.** Elles ne portent pas de delta
  calculé.
