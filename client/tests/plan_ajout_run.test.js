// Ajout de comptes par run : couches d'empilement, tri et filtres.
//
// Vécu en application : la fenêtre d'ajout ouverte depuis le Plan de charge
// était invisible — l'écran plein du plan la recouvrait. Il fallait la fermer
// pour voir ce qu'elle affichait.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { chargerApp } = require("./dom_shim");

const CSS = fs.readFileSync(path.join(__dirname, "..", "src", "styles.css"), "utf8");

/** Le corps de la règle déclarée pour un sélecteur, tel qu'écrit dans la feuille. */
function regle(selecteur) {
  const bloc = CSS.split(selecteur + " {")[1];
  assert.ok(bloc, `sélecteur « ${selecteur} » introuvable dans styles.css`);
  return bloc.split("}")[0];
}

/** Le z-index déclaré pour un sélecteur, tel qu'écrit dans la feuille. */
function couche(selecteur) {
  const m = regle(selecteur).match(/z-index:\s*(\d+)/);
  assert.ok(m, `pas de z-index sur « ${selecteur} »`);
  return Number(m[1]);
}

test("les cases à cocher de la fenêtre d'ajout échappent au style des champs de saisie", () => {
  // Vécu en application : les cases étaient dans le DOM — les tests de tri et
  // de filtres les comptaient — mais invisibles et inatteignables à l'écran.
  // `#modal input { display: block; width: 100% }` habille les champs de
  // saisie des modales de confirmation ; appliqué à une case à cocher logée
  // dans une colonne qui s'ajuste à son contenu, il la réduit à quelques
  // pixels. Le faux DOM ne calculant aucun style, seul un test de la feuille
  // elle-même peut retenir cette règle.
  const cases = regle('#modal input[type="checkbox"]');
  assert.match(cases, /width:\s*auto/, "la case ne doit pas suivre la largeur du champ de saisie");
  assert.match(cases, /display:\s*inline-block/, "ni son display: block");

  // La règle générique doit rester plus faible : elle est ciblée par un
  // sélecteur d'attribut, donc plus spécifique — on vérifie qu'elle existe
  // toujours, sans quoi ce test ne protègerait plus rien.
  assert.match(regle("#modal input"), /width:\s*100%/, "la règle générique a disparu");
});

test("la modale s'empile au-dessus de l'écran plein du plan", () => {
  // On teste l'ORDRE, pas les valeurs : un test sur « z-index: 70 » casserait
  // au prochain réglage sans rien signaler d'utile.
  const settings = couche("#settings-backdrop");
  const plan = couche("#plan-screen");
  const modale = couche("#modal-backdrop");
  const splash = couche("#splash");

  assert.ok(settings < plan, `réglages (${settings}) doit rester sous l'écran du plan (${plan})`);
  assert.ok(plan < modale, `l'écran du plan (${plan}) doit rester sous la modale (${modale})`);
  assert.ok(modale < splash, `la modale (${modale}) doit rester sous le splash (${splash})`);
});

/** Candidats aux statuts contrastés, tels que `plan_candidats_run` les rend.
 *
 *  Le jeu est calibré pour que le test des filtres combinés TOMBE dès qu'un des
 *  trois filtres disparaît : CF-D n'est écarté que par la plateforme, CF-E que
 *  par le statut CTC, CF-C que par PPF. Avec les trois premiers seuls, retirer
 *  le filtre plateforme ne changeait rien — CF-B, le seul autre, étant déjà
 *  écarté par son statut.
 *
 *  CF-F porte les deux singularités que rien d'autre n'expose : un statut CTC
 *  VIDE, seul cas où le filtre doit traduire l'option `(vide)` en chaîne vide,
 *  et un jour de cycle à deux chiffres, seul cas où un tri en texte se voit
 *  (`"12" < "5"`). Sans lui, ces deux branches passent sans être éprouvées. */
const CANDIDATS = [
  { cf: "CF-A", raison_sociale: "ALPHA SARL", jj: 5, pa: "Cegedim",
    eligible: true, participant: "0225:1", ctc_status: "ready", ppf_usable: true },
  { cf: "CF-B", raison_sociale: "BETA SAS", jj: 1, pa: "SAGE",
    eligible: false, participant: "0225:2", ctc_status: "later", ppf_usable: true },
  { cf: "CF-C", raison_sociale: "GAMMA SCI", jj: 5, pa: "Cegedim",
    eligible: false, participant: "0225:3", ctc_status: "ready", ppf_usable: false },
  { cf: "CF-D", raison_sociale: "DELTA SA", jj: 1, pa: "SAGE",
    eligible: true, participant: "0225:4", ctc_status: "ready", ppf_usable: true },
  { cf: "CF-E", raison_sociale: "EPSILON SNC", jj: 5, pa: "Cegedim",
    eligible: false, participant: "0225:5", ctc_status: "later", ppf_usable: true },
  { cf: "CF-F", raison_sociale: "ZETA EURL", jj: 12, pa: "Cegedim",
    eligible: false, participant: "0225:6", ctc_status: "", ppf_usable: true },
];

/** Les comptes d'une liste rendue par le realm du shim, ramenés côté Node :
 *  `deepEqual` compare les prototypes, et un tableau né dans la VM n'a pas
 *  celui de Node. */
const comptes = (liste) => Array.from(liste, (c) => c.cf);

test("le tri par colonne réordonne la liste", () => {
  const ctx = chargerApp();
  const lignes = comptes(ctx.app.trierCandidats(CANDIDATS, "cf", false));
  assert.deepEqual(lignes, ["CF-F", "CF-E", "CF-D", "CF-C", "CF-B", "CF-A"],
    "tri descendant sur le compte");
});

test("le tri sur le jour de cycle est numérique, pas alphabétique", () => {
  // `jj` est la seule colonne chiffrée de la fenêtre. Comparée en texte, elle
  // classerait `12` avant `5` et l'utilisateur lirait un ordre faux sur la
  // colonne qui décide de ce qu'un run peut facturer.
  const ctx = chargerApp();
  const jjs = Array.from(ctx.app.trierCandidats(CANDIDATS, "jj", true), (c) => c.jj);
  assert.deepEqual(jjs, [1, 1, 5, 5, 5, 12]);
});

test("les filtres se combinent", () => {
  const ctx = chargerApp();
  // Plateforme ET statut CTC actifs en même temps : sans le filtre plateforme
  // CF-D (SAGE, ready) passerait, sans le filtre CTC ce serait CF-E (later).
  const out = ctx.app.filtrerCandidats(CANDIDATS, { texte: "", pa: "Cegedim", ctc: "ready", ppf: "" });
  assert.deepEqual(comptes(out), ["CF-A", "CF-C"]);
  const strict = ctx.app.filtrerCandidats(CANDIDATS, { texte: "", pa: "Cegedim", ctc: "ready", ppf: "oui" });
  assert.deepEqual(comptes(strict), ["CF-A"], "le filtre PPF doit encore réduire");
});

test("le filtre CTC « (vide) » retient les statuts vides", () => {
  // L'option porte le libellé `(vide)`, le candidat porte la chaîne vide : le
  // filtre doit traduire. Sans cela l'option existe mais ne rend jamais rien.
  const ctx = chargerApp();
  assert.deepEqual(
    comptes(ctx.app.filtrerCandidats(CANDIDATS, { texte: "", pa: "", ctc: "(vide)", ppf: "" })),
    ["CF-F"]);
});

test("la recherche porte sur le compte et la raison sociale", () => {
  const ctx = chargerApp();
  assert.deepEqual(
    comptes(ctx.app.filtrerCandidats(CANDIDATS, { texte: "beta", pa: "", ctc: "", ppf: "" })),
    ["CF-B"]);
  assert.deepEqual(
    comptes(ctx.app.filtrerCandidats(CANDIDATS, { texte: "cf-c", pa: "", ctc: "", ppf: "" })),
    ["CF-C"], "la recherche ignore la casse");
});

/** Toutes les cases à cocher du sous-arbre, en profondeur d'abord. */
function casesACocher(noeud, out = []) {
  if (typeof noeud !== "object" || noeud === null) return out;
  if (noeud.attrs?.type === "checkbox") out.push(noeud);
  for (const enfant of noeud.children ?? []) casesACocher(enfant, out);
  return out;
}

test("les cases de la fenêtre s'ouvrent décochées", async () => {
  // `h()` pose les attributs par setAttribute, et `checked="false"` COCHE la
  // case : l'état d'une case ne peut venir que de la propriété `checked`. Sans
  // ce détour, la fenêtre s'ouvre avec TOUS les comptes sélectionnés.
  const ctx = chargerApp();
  ctx.repondreAux((cmd) => (cmd === "plan_candidats_run" ? CANDIDATS : null));
  await ctx.app.ouvrirAjoutRun(
    { num: "R3", jjs: [1, 5], exclu: false, ecart: null, detail: { mep_id: 2 } },
    { date: "2026-09-08" });

  const cases = casesACocher(ctx.$("modal"));
  assert.equal(cases.length, CANDIDATS.length + 1,
    "une case par candidat, plus celle qui coche tout");
  assert.ok(cases.every((c) => !c.checked),
    `aucune sélection n'a été faite, ${cases.filter((c) => c.checked).length} case(s) sont pourtant cochées`);
});

/** `PlanApercu` minimal : deux runs retenus, un écarté, tel que la timeline
 *  le reçoit. Les blocs que `renderPlanParam` traverse ensuite (entonnoir,
 *  stock, plateformes) sont là pour que le rendu aille jusqu'au bout. */
function apercuTimeline() {
  const jour = (date, run) => ({
    date, jour_semaine: "mar", weekend: false, ferie: null, jalons: [], runs: [run],
  });
  const detail = { run_num: "", run_date: "", jjs: [], mep_id: 1, mep_date: "2026-08-03",
                   vise: 40, report_entrant: 0, stock: 999, place: 40, reliquat: 0 };
  return {
    funnel: { lignes: 100, cf_distincts: 100, jj_valide: 100, resolus: 100,
              ctc_ready: 100, ppf_usable: 100, eligibles: 100 },
    timeline: [
      jour("2026-08-11", { num: "3320", jjs: [1, 5], exclu: false, ecart: null, detail }),
      jour("2026-09-10", { num: "3327", jjs: [1, 5], exclu: false, ecart: null, detail }),
      jour("2026-10-24", { num: "3342", jjs: [20], exclu: true, ecart: "exclu", detail: null }),
    ],
    stock_jj: [{ jj: 1, comptes: 700, couvert: true }],
    plateformes: [], avertissements: [], meps: ["2026-08-03"],
    cible: 1000, total: 1000, geles: 0, epingles: 0, retires: 0,
  };
}

/** Tous les boutons « + Ajouter » de la timeline rendue. */
function boutonsAjout(noeud, out = []) {
  if (typeof noeud !== "object" || noeud === null) return out;
  if (noeud.className === "tl-add-btn") out.push(noeud);
  for (const enfant of noeud.children ?? []) boutonsAjout(enfant, out);
  return out;
}

/** Timeline rendue, avec ou sans plan enregistré. */
function timeline(genere) {
  const ctx = chargerApp();
  const p = ctx.evaluer("plan");
  p.apercu = ctx.evaluer(`(${JSON.stringify(apercuTimeline())})`);
  p.genere = genere;
  ctx.app.renderPlanParam();
  assert.deepEqual(ctx.plaintes, [], "le rendu de la timeline doit être propre");
  return boutonsAjout(ctx.$("plan-param"));
}

test("« + Ajouter » n'apparaît qu'avec un plan enregistré", () => {
  // `plan_ajouter` retouche le plan PERSISTÉ : sans plan, le bouton est inerte
  // et l'utilisateur ne récolte qu'un bandeau rouge. Or l'aperçu — donc la
  // timeline — existe dès la saisie de la fenêtre FUT, bien avant « Générer ».
  assert.equal(timeline(false).length, 0,
    "aucun plan enregistré : la timeline ne doit offrir aucune action d'ajout");
  assert.equal(timeline(true).length, 2,
    "plan enregistré : une action par run retenu, aucune sur le run écarté");
});
