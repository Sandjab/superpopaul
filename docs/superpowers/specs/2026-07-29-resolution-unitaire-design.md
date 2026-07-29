# Résolution d'un adressage unitaire — spec de conception

Spec de conception — 2026-07-29.
Maquette validée : `docs/superpowers/maquettes/2026-07-29-resolution-unitaire.html`.

## 1. Objectif

Une **loupe dans l'en-tête**, à côté de ⚙, ouvre une modale où l'on saisit un
adressage. L'application le résout contre les trois sources qu'elle connaît —
le **réseau Peppol**, l'**annuaire Peppol**, l'**annuaire PPF** — et restitue
les champs **avec les noms et les valeurs de l'export**.

Usage visé : le diagnostic. « Pourquoi ce compte n'est-il pas éligible ? »
devait jusqu'ici passer par un fichier d'une ligne et un run complet.

Fonctionnalité **CLIENT-ONLY** : aucune parité avec `cli/popaul.py`.

## 2. Décisions arbitrées avec l'utilisateur

| Sujet | Décision |
|---|---|
| Écriture en base | **Aucune.** Consultation seule : `resolutions` n'est pas touchée. Consulter un compte ne le retire jamais du périmètre d'un run futur (les modes s'appuient sur cette table pour savoir ce qui reste à faire). |
| Transport | **Le mode configuré** (API ou direct), via le client courant. Un verdict obtenu par un autre transport que celui du run ne lui serait pas comparable. |
| Présentation | **Modale**, comme ⚙. Pas de tiroir : les trois étapes n'ont jamais eu à céder de la largeur. |
| Icône | 🔍 **monochrome**, même graisse et même hover que ⚙ — deux outils de même rang. |
| Infobulles | **Chaque ligne** de résultat porte sa définition, **prise dans `docs/legende_champs.md`**. Aucun texte réécrit pour l'occasion : deux sources divergeraient au premier changement. |
| Champs de vedette | `ctc_status` et `ppf_usable` : libellé humain sous le nom technique, valeur agrandie, couleur du verdict. Ce sont eux qui décident de l'éligibilité. |
| Couleurs du verdict | **Quatre**, pas deux : `ready` vert, `later` vert éteint (`--green-later`), `expired` rouge, indéterminé ambre. Écart assumé au « vert ou rouge » : `later` basculera seul le jour de l'activation, le peindre en rouge dirait « disqualifié ». Reprend la palette que le cockpit applique déjà à ces mêmes états. |
| Historique des consultations | **Hors scope.** On peut enchaîner les adressages sans fermer la modale ; rien n'est mémorisé. |

## 3. Le principe : ne rien réinventer

La valeur de l'écran tient à une promesse : **ce qu'il affiche est ce que le
fichier de sortie contiendrait pour cette ligne**. Elle n'est tenable que si le
chemin est le même, pas seulement s'il est décrit comme tel.

```
saisie ──pid::canonical──▶ forme canonique
                              │
                              ├──▶ client courant (API|direct) ──▶ ApiItem
                              │        └─ resolver::to_resolution ──▶ store::Resolution
                              │               └─ output::ctc_status ──▶ ready|later|expired|""
                              │
                              └──directory::parse_0225_value──▶ valeur nue 0225
                                       ├──▶ store::directory_present  ──▶ in_directory
                                       └──▶ store::ppf_flags          ──▶ 4 champs PPF
```

Conséquence pour l'implémentation : **`resolver::to_resolution` passe de privé à
`pub(crate)`**. C'est la seule modification d'un module existant. Reconstruire
une conversion parallèle donnerait deux vérités à maintenir — exactement ce que
la fonctionnalité cherche à éviter.

`output::ctc_status` est déjà `pub(crate)` et déjà réutilisé ainsi par
`securisation_from_scan` (« parité colonne CSV ») : le motif est établi.

## 4. Les trois indéterminations

Une source qui ne peut pas répondre ne répond **jamais `false`**. « Je ne sais
pas » et « non » sont deux réponses différentes, et les confondre ferait lire un
constat rassurant là où il n'y en a pas — même discipline que
`avertissement_ppf_cumulatif`.

| Cas | Détection | Affiché |
|---|---|---|
| Annuaire Peppol jamais chargé | `store::peppol_directory_status()` → `None` | « annuaire jamais chargé » |
| Annuaire PPF vide | `store::ppf_summary().distinct_addr == 0` | « annuaire vide » |
| Adressage non-0225 | `directory::parse_0225_value()` → `None` | « hors périmètre des annuaires (0225) » |

Le troisième cas est **structurel** : les deux annuaires sont français et
indexés sur la valeur nue 0225. Un adressage d'un autre ICD n'y est pas
« absent », il n'y est pas cherchable. C'est `filter_map` qui l'élimine
silencieusement dans le chemin de masse (`commands.rs`) ; ici il doit se dire.

Ces trois cas concernent aussi le champ de vedette `ppf_usable` : ni vert ni
rouge, ambre.

## 5. Contrat Rust

Nouvelle commande dans `commands.rs`, nouveau module `unitaire.rs` pour la
logique pure (assemblage et verdicts) — `commands.rs` dépasse 2 800 lignes et
n'a pas besoin d'une responsabilité de plus.

```rust
#[tauri::command]
pub async fn resoudre_adressage(
    saisi: String,
    state: State<'_, AppState>,
) -> Result<ResolutionUnitaire, String>;
```

Le type de retour encode l'invariant plutôt que de le documenter : une source
répond **ou** se tait, jamais les deux, jamais ni l'un ni l'autre.

```rust
#[derive(Serialize)]
pub struct ResolutionUnitaire {
    pub saisi: String,        // tel que tapé, trim
    pub canonique: String,    // pid::canonical
    pub mode: &'static str,   // "api" | "direct"
    pub reseau: Reseau,
    pub annuaire_peppol: Annuaire,
    pub ppf: Ppf,
}

#[derive(Serialize)]
#[serde(tag = "etat", rename_all = "snake_case")]
pub enum Reseau {
    Repond { champs: ChampsReseau, latence_ms: u64 },
    Echec  { message: String },     // l'erreur ApiError, telle quelle
}

#[derive(Serialize)]
#[serde(tag = "etat", rename_all = "snake_case")]
pub enum Annuaire {
    Repond { in_directory: bool },
    Muette { raison: Muette },
}
// idem Ppf { Repond { annuaire_ppf, ppf_active, pdp_definie, ppf_usable } | Muette }

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Muette { AnnuaireNonCharge, AnnuaireVide, HorsPerimetre0225 }
```

`ChampsReseau` porte les 8 champs de la source réseau
(`in_peppol`, `pa_code`, `pa_name`, `pa_country`, `ubl_extended`,
`ctc_activation`, `ctc_expiration`, `ctc_status`) plus la `note` diagnostique du
résolveur, déjà utile aujourd'hui (« ServiceGroup HTTP 403 on … » quand le
catalogue SMP est illisible).

**Un échec réseau n'est pas une erreur de commande** : la commande rend `Ok`
avec `Reseau::Echec`, pour que les annuaires locaux — qui ne dépendent pas du
réseau — restent affichés. Seule une saisie vide rend `Err`.

## 6. IHM

`index.html` : un bouton `#btn-resolve` dans `.cfg-btns`, avant `#btn-settings`.
`app.js` : ouverture par le helper `modal()` existant, contenu construit avec
`h()` — **jamais d'innerHTML**, les valeurs viennent du réseau et d'un CSV.

Le champ prend le focus à l'ouverture ; `Entrée` déclenche la résolution ;
le bouton est désactivé sur saisie vide et pendant l'appel. La forme canonique
retenue est affichée sous la saisie — c'est souvent là qu'est l'explication d'un
« absent de Peppol » (un SIREN nu devient `…::0225:<siren>`).

Les infobulles sont posées en `title` sur la ligne entière (`tr[title]`,
`cursor:help`) : viser un nom de huit caractères est pénible.

## 7. Tests

**Rust — `unitaire.rs`, sans UI ni réseau** (assemblage à partir de valeurs déjà
obtenues) :
- les trois indéterminations rendent `Muette` avec la bonne raison, jamais `false` ;
- un adressage non-0225 rend `HorsPerimetre0225` pour **les deux** annuaires ;
- `ctc_status` est bien celui d'`output::ctc_status` pour un même `Resolution`
  (le test doit échouer si quelqu'un recalcule l'état localement) ;
- un échec réseau laisse les deux annuaires renseignés.

**Rust — commande, wiremock** : la forme canonique envoyée à l'API est celle de
`pid::canonical` pour une saisie de SIREN nu.

**JS — `client/tests/`** : la modale n'appelle pas la commande sur saisie vide ;
un `Reseau::Echec` affiche le message ET les sections d'annuaire ; une valeur
`Muette` ne s'affiche pas « false ».

## 8. Hors scope

- **Aucune écriture** : ni `resolutions`, ni historique de consultation.
- **Pas de résolution par lot** depuis cette modale (c'est le rôle d'un run).
- **Pas d'export** du résultat unitaire.
- **Pas de reprise** du chemin moteur (retries, breaker, télémétrie) : la
  consultation est un appel simple, un échec s'affiche et se relance à la main.
