# Plan de charge FUT (Runs de Facturation) — design

Date : 2026-07-25

## Contexte

Le co-repo `../peppolstat` porte une chaîne « FUT » (montée en charge de la facturation
électronique) en cinq fichiers : `fut_config.py` (contrat de configuration),
**`fut_plan.py`** (cœur métier : pool d'éligibilité, quotas, rampe, allocation, gel),
`fut_batch.py` (CLI d'orchestration), `fut_planner.html` (IHM navigateur d'édition du
calendrier) et `fut_report.py` (rapport HTML). Le modèle : un **calendrier de runs de
facturation** consomme des comptes de facturation (CF) selon leur jour de cycle (JJ),
par paliers successifs de mise en production (MEP), en respectant une rampe de montée
en charge et des quotas par plateforme.

L'utilisateur veut ces fonctionnalités dans superpopaul, en interaction directe avec la
base SQLite du client.

**Le décalage de modèle qui structure tout le chantier** : superpopaul est orienté
**adressage** (`resolutions`, PK `participant`, en upsert, sans historique ; un run de
résolution n'existe qu'en mémoire via `commands.rs::LastRun`), alors que le plan de charge
est orienté **CF**. La table `brm` de peppolstat (`CF_ID`, `ADRESSAGE_ID`,
`ACTG_CYCLE_DOM`) n'a aucun équivalent : c'est le trou à combler. Un adressage porte N CF,
avec N jours de cycle potentiellement différents.

Correspondance des référentiels (établie par lecture des deux bases) :

| peppolstat | superpopaul | Statut |
|---|---|---|
| `calliope` (IDENTIFIANT, MOTIF_PRESENCE, UTILISE_PDP_FICTIVE) | `ppf_directory` | équivalent |
| `peppol` | `peppol_directory` | équivalent |
| `peppol_resolution` | `resolutions` (+ `extended_ctc_fr`, fenêtre CTC) | équivalent, plus riche |
| `ap_doctype` (sondage des AP) | — | remplacé par `ctc_status` par adressage |
| `brm` | — | **à combler** |

## Objectif

Permettre de définir un **plan de charge** sur la base d'un calendrier de Runs de
Facturation, puis de l'appliquer au résultat d'une résolution complète pour produire la
liste des comptes de facturation à activer, MEP par MEP.

Périmètre livré en deux temps :

- **v1** — moteur complet + IHM à deux onglets (paramétrage tabulaire + récapitulatif
  des CF avec retouche manuelle) + rapport minimal + fichiers livrables.
- **v2** — timeline calendaire, rampe manuelle run par run, courbes du rapport.

## Décisions validées (brainstorm 2026-07-25)

1. **Éligibilité** : `ctc_status == ready` **ET** `ppf_usable`.
   *Pas* `ppf_active` : `usable` exige motif actif **et** `pdp_fictive = 0` sur la **même**
   ligne, seul pendant exact du critère peppolstat (motif + PDP réelle sur la ligne
   représentative Calliope). `ready` implique déjà `extended_ctc_fr = true`
   (`output::ctc_status` renvoie `""` sinon).
2. **Motifs actifs** : réglage **global** (`Settings.ppf.active_motifs`), non surchargeable
   par plan.
3. **CF et JJ** : colonnes du **CSV d'entrée courant**. Aucune ingestion séparée.
4. **Doublons CF** : dédoublonnage silencieux si la ligne est strictement identique ;
   **refus fort** si deux lignes portent le même CF avec des JJ divergents.
5. **JJ absent ou invalide** : compté et affiché dans le funnel d'éligibilité, jamais
   écarté en silence.
6. **Persistance** : le pool éligible est **recalculé à la volée** (aucune table) ; seul
   le **plan** est stocké, et il est **auto-porteur** (embarque CF, adressage, JJ, PA) car
   le gel doit survivre à un changement de fichier d'entrée. Aucun statut n'est figé —
   doctrine `store.rs` : « on stocke les dates, jamais l'état ».
7. **Paramétrage du plan** : **en base**, dans la **même transaction** que les lignes.
   Un seul plan actif. Motif : le gel lie indissociablement un plan à ses paramètres ;
   deux artefacts séparés divergeraient en silence.
8. **Pas de reproduction des plans peppolstat existants** → le `random.Random(seed).shuffle`
   n'est pas porté (Mersenne Twister CPython non reproductible en Rust) : tri déterministe
   sur hash stable seedé.
9. **Vocabulaire** : « **Run de Facturation** » et « **Run de Résolution** », partout
   (SQL, Rust, UI). Jamais « run » seul.
10. **Client-only** : pas de parité `popaul.py`, pas de serveur (même statut que
    l'annuaire PPF).
11. **Livrable** : CF nus, un par ligne, cumulatif par MEP. **Pas de manifest**.
12. **Rapport** : distinct du rapport de run existant.
13. **IHM** : écran de **plein niveau** (pas une 4ᵉ étape du stepper : le plan est un
    atelier itératif, pas un tunnel, et il lui faut un layout deux colonnes). Point
    d'entrée « Établir un plan de charge → » en fin d'étape 3. Fenêtre Tauri secondaire
    écartée (cf. bug drag-drop multi-écran connu). **Deux onglets** dans cet écran :
    *Paramétrage* et *Comptes de facturation* — les allers-retours entre « je change la
    rampe » et « je regarde ce que ça donne » sont constants, deux écrans séparés les
    rendraient pénibles.
14. **Mapping des colonnes éditable depuis l'écran de plan** (bloc « Colonnes » en tête
    du panneau latéral), en plus de l'étape 2. Même réglage `InputConfig` : une seule
    source de vérité, deux points d'accès.
15. **Quotas par plateforme affichés**, avec la mention explicite qu'ils sont une
    **cible souple** — quand le volume d'un run dépasse les quotas restants des
    plateformes présentes, le volume prime. Sans cette mention, l'écart final passerait
    pour un défaut.
16. **Retouche manuelle du plan** : ajout, déplacement et retrait de CF, avec
    **épinglage** des lignes retouchées et **retrait tracé** (motif obligatoire).
    Détail des règles en section « Retouche manuelle ».

## Vocabulaire

| Terme | Sens |
|---|---|
| **CF** | Compte de facturation. Unité planifiée. |
| **JJ** | Jour du cycle de facturation d'un CF (1–31). |
| **Run de Facturation** | Date à laquelle la facturation tourne, pour une liste de JJ. |
| **Run de Résolution** | Un lot d'appels API superpopaul (sens historique du projet). |
| **MEP** | Mise en production : date à laquelle un lot de CF est déclaré. |
| **Rampe** | Profil des volumes de premières factures par Run de Facturation. |
| **Pool éligible** | CF satisfaisant les critères durs, calculé à la volée. |
| **Gelé** | Ligne d'une MEP passée : conservée à l'identique à la régénération. |
| **Épinglé** | Ligne issue d'une retouche manuelle : survit à la régénération. |
| **Retiré** | Ligne écartée à la main, conservée en base avec son motif, exclue des fichiers et du re-tirage. |

## Architecture

Trois couches, empilées, toutes nécessaires :

```
csv_io multi-colonnes
  └─ Pool éligible depuis scan (aucune persistance)
       └─ calendrier.rs + plan.rs (calcul pur, TDD, sans UI)
            └─ Commandes Tauri + IHM
```

Le calcul du pool suit le pattern maison déjà en place trois fois
(`coverage_from_scan`, `securisation_from_scan`, `repartition_from_scan`) : un scan du
CSV, des jointures via `load_map` / `ppf_flags`, un agrégat pur. Aucune donnée dérivée
n'est stockée.

**Gain de l'intégration** : `fut_planner.html` ré-implémente en JS `largest_remainder`,
`build_ramp`, `complete_meps`, `usable_runs` et une *approximation gloutonne* de
`allocate_runs` (commentée « ~0,1 % d'écart, `fut_batch` fait foi »). Ici l'IHM appelle une
commande et affiche le **vrai** plan. La duplication disparaît au lieu d'être recréée,
conformément à la règle projet « l'UI n'a aucune logique métier ».

## Modèle de données

### Mapping des colonnes d'entrée

`config::InputConfig` gagne trois champs, tous **optionnels** (l'app reste pleinement
utilisable sans plan de charge) et rétro-compatibles (`#[serde(default)]`) :

```rust
pub struct InputConfig {
    // … pid_column, record_label existants
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cf_column: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub jj_column: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raison_sociale_column: String,   // informatif, facultatif même en plan
}
```

L'écran de plan de charge est indisponible tant que `cf_column` et `jj_column` ne sont pas
renseignées, avec un message explicite renvoyant à l'étape 2 (Format).

`csv_io::read_column` ne lit qu'une colonne : ajouter
`read_columns(path, meta, &[&str]) -> Result<Vec<Vec<String>>, String>` (un seul passage,
erreur nommant la première colonne absente).

### Pool éligible (en mémoire)

```rust
// plan.rs
pub struct CfCandidat {
    pub cf: String,
    pub participant: String,     // forme canonique (longue)
    pub jj: u8,                  // 1..=31
    pub raison_sociale: String,  // "" si colonne non mappée
    pub pa: String,              // repartition::pa_key (pa_name, repli pa_code)
    pub in_directory: bool,      // bonus de tri
    pub resolved_at: i64,        // bonus de tri (fraîcheur)
}

pub struct Funnel {
    pub lignes: u64,
    pub cf_distincts: u64,
    pub jj_valide: u64,
    pub resolus: u64,        // présents en base, api_status ok
    pub ctc_ready: u64,
    pub ppf_usable: u64,
    pub pa_exclue: u64,      // retirés par exclusion explicite de PA
    pub eligibles: u64,
}
```

Le funnel est affiché tel quel : chaque marche perdue est visible (règle 5 de la section
Décisions). Un CF dont l'adressage n'est pas en base tombe à l'étape `resolus` — c'est le
cas normal si la résolution est partielle, et il doit se lire, pas se deviner.

### Calendrier

```rust
// calendrier.rs
pub struct RunFacturation {
    pub num: String,
    pub date: NaiveDate,
    pub jjs: Vec<u8>,        // triés, dédoublonnés, 1..=31
    pub exclu: bool,
}

pub struct Rampe {
    pub forme: Forme,        // Plate | Lineaire | Geometrique { raison: f64 } | Manuelle
    pub pilote: Option<Pilote>,   // { runs: u32, cf_par_run: u32 }
}
```

Règles portées telles quelles de `fut_config` / `fut_plan` :

- **Runs utilisables** : non exclus, dans `[fut_start, fut_end]`, **strictement**
  postérieurs à la première MEP (un run le jour même d'une MEP n'est pas utilisable).
- **Complétion des MEP** : les dates fournies sont conservées ; complétion jusqu'à
  `mep_count` par équirépartition sur `[fut_start, fut_end)`. Une MEP **auto** sans run
  utilisable après elle est ramenée à la veille du dernier run candidat ; une MEP
  **fournie** dans ce cas est gardée avec avertissement.
- **MEP de rattachement** d'un run : la dernière MEP **strictement** antérieure.

### Tables SQLite

Ajoutées au `SCHEMA` de `store.rs`. Noms à confirmer à l'implémentation
(`plan_cf` / `plan_meta` proposés, alignés sur `ppf_directory` / `ppf_files`).

```sql
CREATE TABLE IF NOT EXISTS plan_cf (
  cf              TEXT PRIMARY KEY,
  participant     TEXT NOT NULL,   -- forme STOCKÉE (0225 nu, cf. store::to_stored)
  jj              INTEGER NOT NULL,
  raison_sociale  TEXT,
  pa              TEXT,
  mep_id          INTEGER NOT NULL,
  mep_date        TEXT NOT NULL,
  run_num         TEXT NOT NULL,
  run_date        TEXT NOT NULL,
  origine         TEXT NOT NULL,    -- 'auto' | 'couverture' | 'manuel'
  in_directory    INTEGER,
  resolved_at     INTEGER,
  frozen          INTEGER NOT NULL DEFAULT 0,
  planned_at      INTEGER NOT NULL,
  retire_le       INTEGER,          -- NULL = ligne active
  retire_motif    TEXT              -- non vide dès que retire_le l'est
);
CREATE INDEX IF NOT EXISTS idx_plan_cf_mep ON plan_cf(mep_id);

CREATE TABLE IF NOT EXISTS plan_meta (
  id           INTEGER PRIMARY KEY CHECK (id = 1),
  fichier      TEXT NOT NULL,      -- nom du CSV source
  hash         TEXT NOT NULL,      -- SHA-256 du contenu (sha2, déjà dépendance)
  genere_le    INTEGER NOT NULL,
  params_yaml  TEXT NOT NULL       -- serde_yaml, déjà dépendance
);
```

Notes de conception :

- **Pas de `first_bill_date`.** Chez peppolstat cette colonne vaut *toujours* `run.date`
  (vérifié : les deux appels à `take()` passent `run.date`), y compris pour les
  remplissages de couverture. Colonne redondante, supprimée.
- **`origine` absorbe `coverage_fill`.** Un remplissage de couverture *est* une origine
  d'affectation ; en faire un booléen à part obligerait à un second booléen dès qu'une
  troisième origine apparaît (ici : `manuel`). Une seule colonne, trois valeurs.
- **`participant` en forme stockée** (0225 nu) pour rester joignable directement à
  `ppf_directory` / `peppol_directory` sans `substr`, comme `resolutions` depuis le
  24/07. La conversion reste confinée à la frontière SQL.
- **`params_yaml`** plutôt qu'un blob JSON : `serde_yaml` est déjà la sérialisation de
  configuration du projet (`config.rs`). Il porte fenêtre, MEP fournies, `mep_count`,
  cible, rampe, seed, runs exclus, PA exclues, **et le calendrier importé** — ce qui évite
  une troisième table pour quelques dizaines de runs.
- **Écriture transactionnelle** : `DROP` + recréation des lignes + `plan_meta` dans une
  seule transaction (même discipline que `replace_peppol_directory`, dont le commentaire
  note que « l'horodatage ne peut pas diverger du contenu »).

## Algorithmes

### Tri de priorité (remplace le shuffle seedé)

Chez peppolstat : `shuffle(seed)` puis trois tris stables successifs
(`resolved_at` desc, puis `-in_annuaire`, `-strict`). Ici, **une seule clé composite** :

```
(in_directory desc, resolved_at desc, fnv1a(seed, cf) asc)
```

Déterministe, reproductible, réglable par seed, sans dépendance. Le bonus `ext_strict`
(socle PASR, table `ap_doctype`) n'a pas d'équivalent en base superpopaul : il disparaît
du tri, ce qui est assumé.

### Quotas par plateforme

Port de `global_quotas` : proportionnels au pool éligible par PA (plus forts restes),
**plancher 1** (toute PA ayant ≥ 1 CF éligible doit être représentée), **plafond = stock**,
avec redistribution itérative de l'excédent. Clé PA = `repartition::pa_key` (nom, repli
sur le code) — la fonction existe déjà et est testée.

### Rampe

Port de `build_ramp`, y compris la subtilité du **socle** : quand un pilote est actif
(P premiers runs à V CF), chaque run post-pilote démarre à V et la forme ne répartit que
le surplus — la rampe prolonge le pilote sans jamais redescendre sous V. Si la cible ne
suffit pas (`cible < N·V`), le socle est abandonné, la forme pure s'applique et un
avertissement est émis (`ramp_pilote_infaisable`).

Somme des volumes **exactement** égale à la cible dès que la liste de runs est non vide.

### Allocation

Port de `allocate_runs` : parcours chronologique des runs utilisables ; le volume non
absorbable (stock insuffisant sur les JJ du run) **glisse** au run suivant ; reliquat
final → avertissement, pas une erreur. Les quotas PA sont des cibles **souples** : si le
volume d'un run dépasse les quotas restants des PA présentes, le volume prime.

**Couverture** : toute PA du pool non servie reçoit 1 CF hors quota sur le **premier** run
chronologique couvrant le JJ d'un de ses candidats (`origine = 'couverture'`). Si aucun run
ne couvre, avertissement nommant la PA.

### Régénération : trois ensembles préservés

Régénérer fait `DROP` + réallocation. Trois ensembles échappent au re-tirage, par le même
mécanisme (leurs CF sont retirés du pool des candidats et consomment leur part de la cible) :

| Ensemble | Critère | Motif |
|---|---|---|
| **Gelées** | `mep_date` strictement antérieure à la date de gel | Un lot livré ne bouge pas |
| **Épinglées** | `origine = 'manuel'` | Une retouche ne se perd pas au prochain changement de rampe |
| **Retirées** | `retire_le` non nul | Sinon la rampe replacerait automatiquement un CF écarté à la main |

Sans le deuxième ensemble, une retouche sur une MEP non gelée disparaîtrait **en silence**
au premier changement de paramètre — le mode de panne que la persistance transactionnelle
des paramètres écarte déjà par ailleurs.

Une MEP gelée qui disparaîtrait de la configuration provoque un **refus fort** (les
fichiers étant cumulatifs, un lot déjà livré changerait en silence).

### Retouche manuelle

Ajout, déplacement et retrait de CF depuis l'onglet *Comptes de facturation*. Toute ligne
ajoutée ou déplacée passe en `origine = 'manuel'` et devient épinglée.

1. **Déplacement vers un run ne couvrant pas le JJ du CF** : impossible — et *non proposé*
   plutôt que refusé après coup (le sélecteur ne liste que les runs compatibles). Ce n'est
   pas une préférence : un CF de JJ 12 ne facturera jamais un run traitant les JJ 1 et 5.
2. **Ajout d'un CF non éligible** (CTC `later`/`expired`, PPF non utilisable) : **autorisé**,
   marqué visiblement dans le récapitulatif, avec un avertissement permanent tant qu'il est
   au plan. Cas d'usage assumé : forcer un compte pilote qu'on sait prêt côté PDP.
3. **Ajout d'un CF absent du fichier courant** : refusé — ni JJ ni adressage disponibles.
4. **Retrait** : **autorisé partout, y compris sur une MEP gelée**, avec **motif
   obligatoire** (texte libre non vide). La ligne n'est pas supprimée : elle est conservée
   avec `retire_le` + `retire_motif`, exclue des fichiers, des comptages et du re-tirage,
   et reste consultable via le filtre « retirés » (le retrait est annulable).
   Sur une MEP **gelée**, l'action affiche un avertissement fort : les fichiers étant
   cumulatifs, **un fichier déjà transmis change**. C'est assumé (cas réel : on sait qu'un
   compte va échouer pour une raison hors de notre contrôle), mais ça doit être dit au
   moment de l'action et rester tracé.
5. **Dépassement de la cible** par une retouche : autorisé — la cible est indicative une
   fois le plan produit. Signalé dans le bandeau.

Les retouches sont écrites **immédiatement** en base (pas de brouillon à valider) : elles
portent sur un plan déjà généré, et l'annulation passe par l'action inverse.

### Vérification d'éligibilité

Pas un mode à part comme le `--verify` de peppolstat : le recalcul étant permanent,
l'écran affiche en continu les CF du plan **devenus inéligibles** (ctc expiré, ppf_usable
perdu, ou absents du fichier courant). Information, pas blocage — les MEP passées restent
gelées.

## Contrat d'import `runs.csv`

En-tête obligatoire `DATE_RUN;NUM_RUN;JJS`. Séparateur et encodage via `csv_io::sniff`
(cohérence maison). Une ligne par Run de Facturation :

- `DATE_RUN` : **JJ/MM/AAAA** strict (format de l'intrant fourni par l'équipe facturation) ;
  date inexistante → erreur nommant la ligne ;
- `NUM_RUN` : non vide, unique ;
- `JJS` : entiers 1–31 séparés par `-` (ex. `1-5-15`).

Validation **fail-loud ligne par ligne**, toutes les erreurs remontées ensemble (pas
d'arrêt à la première), messages actionnables affichés tels quels. Deux runs à la même
date → erreur.

## Livrable

Dans le répertoire de sortie résolu (`resolved_out_dir`, comme le reste) :

`<entrée>_plan_mep_<n>_<AAAA-MM-JJ>.txt` — un fichier par MEP, **cumulatif** (MEP 1..n),
CF nus, un par ligne, triés, UTF-8 sans BOM, `\n`. Pas de manifest.

Les lignes **retirées** (`retire_le` non nul) sont exclues, y compris sur une MEP gelée —
c'est précisément l'objet du retrait. Conséquence à assumer : le fichier d'une MEP déjà
transmise peut donc changer d'un tirage à l'autre.

## Rapport

Fichier distinct `<entrée>_plan.html`, à côté de `<entrée>_rapport.html`. Style et helpers
de `report.rs` (thème « Bleu nuit & or », `esc` obligatoire sur toute donnée d'origine CSV
ou SMP).

- **v1** : KPI (CF planifiés, dont gelés, MEP, pool éligible, PA couvertes / PA du pool),
  avertissements, table des MEP et Runs de Facturation (volume, cumul, JJ), répartition
  par PA (plan vs pool).
- **v2** : courbe cumulée déclarés / premières factures, charge par jour civil (premières
  factures + récurrences projetées).

## IHM

Écran de plein niveau à **deux onglets**, layout deux colonnes (paramètres à gauche,
résultats à droite). Point d'entrée : action « Établir un plan de charge → » en fin
d'étape 3, à côté de « Générer le fichier » et « Rapport ».

### Panneau latéral (commun aux deux onglets)

Bloc **Colonnes** en tête — CF, JJ, raison sociale — éditable sur place (décision 14) ;
c'est aussi ce qui débloque l'écran quand le mapping est absent, sans aller-retour vers
l'étape 2. Puis : import `runs.csv`, fenêtre FUT, MEP + `mep_count`, cible, rampe, seed,
et l'action « Générer / Régénérer le plan ».

### Onglet 1 — Paramétrage

Funnel d'éligibilité (chaque marche avec son effectif **et sa perte**) · table des Runs de
Facturation retenus · liste des plateformes avec exclusion, volume éligible et quota
(mention « cible souple ») · avertissements · bloc de résultat listant les fichiers écrits.

Table des runs, cinq colonnes chiffrées : **Visé · Report · Stock JJ · Placé · Reliquat**.
Le report entrant est une colonne distincte du visé : sans elle, un run qui place plus que
son volume de rampe (parce qu'il absorbe le reliquat du précédent) est incompréhensible.

### Onglet 2 — Comptes de facturation

Récapitulatif de toutes les lignes du plan, **filtrable** (MEP, run, plateforme, origine,
statut d'éligibilité courant, retirées) et **triable** sur toutes les colonnes.

Colonnes : CF · adressage · raison sociale · JJ · plateforme · MEP · Run de Facturation ·
origine · état (éligible / devenu inéligible / retiré).

Actions, avec **sélection multiple** (retoucher 40 CF un par un serait un supplice) :
- **Ajouter** — recherche dans les CF du fichier courant absents du plan ; le sélecteur de
  run ne propose que les runs couvrant le JJ du CF ; un CF non éligible est ajoutable mais
  signalé (règle 2) ;
- **Déplacer** — même contrainte de compatibilité JJ ;
- **Retirer** — saisie du motif obligatoire ; avertissement fort si la MEP est gelée ;
- **Annuler un retrait** — depuis le filtre « retirées ».

Marquage visuel : gelé, épinglé, couverture et retiré doivent se distinguer d'un coup
d'œil, sans confusion avec l'or (accent d'action) ni l'orange (avertissement).

### v2

Timeline calendaire (tous les jours de la fenêtre, week-ends, fériés FR calculés + ajoutés,
MEP, bornes), rampe manuelle saisie run par run, aperçu en barres, stock par JJ.

Les **fériés sont purement décoratifs** : vérifié chez peppolstat, `extra_holidays` est
chargé et validé par `fut_config` mais **n'est lu par aucun calcul** de `fut_plan`,
`fut_batch` ni `fut_report`. Ils ne servent qu'à la lecture de la timeline → hors v1, avec
le computus de Meeus à porter côté Rust en v2.

Contraintes projet applicables : **maquette HTML validée avant tout code d'IHM** ;
**jamais d'`innerHTML` avec des données dynamiques** — la couche de rendu de
`fut_planner.html` (concaténation + `esc()` dans `renderTimeline`, `renderJJ`,
`renderRampPreview`) est à réécrire via le helper `h()`.

### Commandes Tauri

| Commande | Rôle |
|---|---|
| `plan_import_runs(path)` | Parse `runs.csv` → calendrier + erreurs |
| `plan_preview(params)` | Funnel, pool, volumes, faisabilité — **calcul réel**, rien d'écrit |
| `plan_generate(params)` | Écrit `plan_cf` + `plan_meta` (1 transaction) + fichiers |
| `plan_load()` | État persisté (paramètres + gel) au retour sur l'écran |
| `plan_status()` | CF du plan devenus inéligibles |
| `plan_lignes(filtres, tri)` | Récapitulatif de l'onglet 2 |
| `plan_candidats(recherche)` | CF du fichier absents du plan, avec runs compatibles |
| `plan_ajouter(cf[], run_num)` | Ajout manuel (épinglé) |
| `plan_deplacer(cf[], run_num)` | Déplacement (épinglé), refus si JJ incompatible |
| `plan_retirer(cf[], motif)` | Retrait tracé, motif obligatoire |
| `plan_annuler_retrait(cf[])` | Réactivation d'une ligne retirée |
| `plan_ecrire_fichiers()` | Réécrit les fichiers par MEP après retouche |

Explorer des scénarios est gratuit : `plan_preview` ne persiste rien. La persistance ne
sert qu'à figer et livrer — c'est ce qui justifie qu'un seul plan actif suffise.

## Tests (TDD, Rust)

`calendrier::tests` :
- runs utilisables : exclusion, bornes de fenêtre, run le jour même d'une MEP écarté ;
- complétion des MEP : équirépartition, MEP auto ramenée à la veille du dernier run,
  MEP fournie sans run postérieur → conservée + avertissement ;
- MEP de rattachement = dernière strictement antérieure ;
- parsing `runs.csv` : en-tête absent, date JJ/MM/AAAA invalide, date inexistante
  (31/02), JJ hors 1–31, numéro dupliqué, deux runs même date, erreurs cumulées.

`plan::tests` :
- funnel : chaque marche (JJ invalide, non résolu, ctc `later`/`expired`, `ppf_usable`
  faux, PA exclue) retire bien ce qu'elle doit et **rien d'autre** ;
- éligibilité : `ppf_active` vrai mais `ppf_usable` faux → **exclu** (le test qui encode
  la décision 1) ;
- doublons CF : ligne strictement identique → dédoublonnée en silence ; JJ divergents →
  erreur nommant le CF ;
- `largest_remainder` : somme exacte, départage déterministe ;
- quotas : plancher 1 par PA, plafond au stock, redistribution de l'excédent ;
- rampe : les quatre formes ; somme = cible ; socle du pilote respecté ; pilote infaisable
  → forme pure + avertissement ;
- allocation : glissement du reliquat, reliquat final → avertissement (pas une erreur),
  couverture d'une PA non servie sur le premier run couvrant, PA sans run couvrant →
  avertissement nommant la PA ;
- déterminisme : deux exécutions à seed égal → plan identique ; seeds différents → plans
  différents ;
- gel : MEP passée conservée à l'identique, CF gelés hors re-tirage, MEP gelée disparue de
  la config → refus fort.

`plan::tests` — retouche manuelle (les tests qui encodent les règles 1 à 5) :
- **régénération** : une ligne `origine = 'manuel'` survit à un changement de rampe (le
  test qui empêche la perte silencieuse) ; une ligne `auto` est bien re-tirée ;
- une ligne **retirée** n'est pas replacée par la régénération suivante (sinon le retrait
  ne tient pas) et n'apparaît dans **aucun** fichier de MEP, gelée comprise ;
- déplacement vers un run ne couvrant pas le JJ → refusé ; la liste des runs compatibles
  ne contient que ceux couvrant le JJ ;
- ajout d'un CF non éligible → accepté et marqué ; ajout d'un CF absent du fichier → refusé ;
- retrait sans motif → refusé ; retrait sur MEP gelée → accepté, tracé, et le fichier
  cumulatif de cette MEP perd bien la ligne ;
- annulation d'un retrait → la ligne redevient active et réapparaît dans les fichiers ;
- gelées + épinglées + retirées consomment leur part de la cible (pas de double compte).

`store` : écriture transactionnelle du plan (échec → aucune ligne, meta inchangée),
`participant` bien stocké en forme nue, relecture fidèle.

`csv_io` : `read_columns` en un passage, erreur nommant la colonne absente.

## Hors-scope

- **v2** : timeline calendaire, fériés, rampe manuelle, courbes du rapport.
- Parité CLI (`popaul.py`) : non concernée, client-only.
- Le serveur (`server/`) : aucun changement.
- Le rapport de run existant, la télémétrie, le chemin de résolution : inchangés.
- Les en-têtes du CSV de sortie (`output::field_name`) : inchangés.
- Reproduction à l'identique de plans peppolstat existants : explicitement abandonnée.
- Multi-plans / scénarios comparés en base : un seul plan actif.

## Fichiers touchés

Nouveaux :
- `client/src-tauri/src/calendrier.rs` — runs, MEP, fenêtre, parsing `runs.csv`.
- `client/src-tauri/src/plan.rs` — funnel, quotas, rampe, allocation, gel.
- `client/src-tauri/src/plan_report.rs` — rapport distinct (ou section dédiée de
  `report.rs`, à trancher à l'implémentation).

Modifiés :
- `client/src-tauri/src/csv_io.rs` — `read_columns`.
- `client/src-tauri/src/config.rs` — `cf_column`, `jj_column`, `raison_sociale_column`.
- `client/src-tauri/src/store.rs` — `plan_cf`, `plan_meta`, écriture transactionnelle,
  chargement du gel.
- `client/src-tauri/src/commands.rs` — commandes `plan_*`, calcul du pool depuis scan.
- `client/src-tauri/src/lib.rs` — déclaration des modules, enregistrement des commandes.
- `client/src/index.html`, `app.js`, `styles.css` — étape 2 (mapping des colonnes),
  point d'entrée en fin d'étape 3, écran de plan de charge.
