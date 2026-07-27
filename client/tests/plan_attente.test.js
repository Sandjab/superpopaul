// Indication de traitement sur l'écran Plan de charge.
//
// Toutes les commandes du plan refont un scan CSV et une jointure SQLite, et
// aucune n'émet d'avancement. Deux réponses distinctes : les gestes voulus font
// parler leur bouton, l'aperçu automatique — qui se relance à chaque frappe —
// marque ses chiffres périmés.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

/** Anti-rebond du recalcul (250 ms) plus une marge, comme plan_rampe. */
const attendreApercu = () => new Promise((r) => setTimeout(r, 320));
/** Laisse tourner les micro-tâches sans avancer d'un tour de minuterie. */
const tick = () => new Promise((r) => setTimeout(r, 0));

/** Promesse que le test résout quand il veut : fige le backend « en cours ». */
function enAttente() {
  let resoudre, rejeter;
  const p = new Promise((ok, ko) => { resoudre = ok; rejeter = ko; });
  // Une promesse rejetée dont personne n'attend encore le résultat ferait
  // sortir Node en `unhandledRejection` : ce `catch` neutre l'en empêche sans
  // masquer le rejet pour le vrai consommateur.
  p.catch(() => {});
  return { p, resoudre, rejeter };
}

/** `PlanApercu` minimal — ce que lisent les rendus déclenchés par un aperçu. */
const APERCU = {
  funnel: { lignes: 10, cf_distincts: 10, jj_valide: 10, resolus: 10,
            ctc_ready: 10, ppf_usable: 10, eligibles: 10 },
  timeline: [], stock_jj: [], plateformes: [], avertissements: [],
  meps: ["2026-09-01"], cible: 10, total: 10, geles: 0, epingles: 0, retires: 0,
};
const GENERATION = { apercu: APERCU, fichiers: [], obsoletes: [] };

const libelle = (n) => String(n?.children?.[0] ?? "");

/** Écran de paramétrage rendu, calendrier saisi pour que l'aperçu parte. */
function ecranPlan() {
  const ctx = chargerApp();
  ctx.evaluer("plan").runs =
    ctx.evaluer(`([{ num: "3320", date: "2026-08-11", jjs: [1], exclu: false }])`);
  ctx.app.renderPlanParam();
  ctx.$("plan-debut").value = "2026-08-01";
  ctx.$("plan-fin").value = "2026-11-30";
  return ctx;
}

// ------------------------------------------------------- gestes explicites

test("le bouton de génération dit ce qu'il fait et se verrouille", async () => {
  const ctx = ecranPlan();
  const attente = enAttente();
  ctx.repondreAux((cmd) => (cmd === "plan_generate" ? attente.p : ctx.evaluer("[]")));

  const avant = libelle(ctx.$("btn-plan-gen"));
  const fini = ctx.app.genererPlan();
  await tick();

  const b = ctx.$("btn-plan-gen");
  assert.equal(b.disabled, true, "le bouton doit être verrouillé pendant le traitement");
  assert.match(libelle(b), /en cours/i, `le libellé doit nommer ce qui se passe : ${libelle(b)}`);
  assert.notEqual(libelle(b), avant);

  attente.resoudre(ctx.evaluer(`(${JSON.stringify(GENERATION)})`));
  await fini;
});

test("après une erreur, le bouton se rouvre quand même", async () => {
  // LE test du rétablissement : en succès, `renderPlanParam` reconstruit le
  // bouton et masquerait un oubli. En échec il n'y a pas de re-rendu — sans
  // `finally`, l'écran reste verrouillé sur un bouton mort et l'utilisateur
  // ne peut plus réessayer.
  const ctx = ecranPlan();
  const attente = enAttente();
  ctx.repondreAux((cmd) => (cmd === "plan_generate" ? attente.p : ctx.evaluer("[]")));

  const avant = libelle(ctx.$("btn-plan-gen"));
  const fini = ctx.app.genererPlan();
  await tick();
  attente.rejeter("fenêtre FUT : la fin doit suivre le début");
  await fini;

  const b = ctx.$("btn-plan-gen");
  assert.equal(b.disabled, false, "le bouton doit se rouvrir malgré l'échec");
  assert.equal(libelle(b), avant, "et retrouver son libellé");
  assert.equal(ctx.$("plan-banner").className, "error", "et l'erreur doit être dite");
});

test("un second geste pendant le traitement ne relance rien", async () => {
  // Le verrou n'est pas décoratif : deux générations concurrentes écriraient
  // les mêmes fichiers en même temps.
  const ctx = ecranPlan();
  const attente = enAttente();
  ctx.repondreAux((cmd) => (cmd === "plan_generate" ? attente.p : ctx.evaluer("[]")));

  const fini = ctx.app.genererPlan();
  await tick();
  // Volontairement NON attendu : tant que la garde n'existe pas, ce second
  // appel part vers un backend figé et ne se résout jamais. L'attendre ferait
  // PENDRE le test au lieu de l'échouer.
  ctx.app.genererPlan();
  await tick();

  assert.equal(ctx.invocations.filter(([c]) => c === "plan_generate").length, 1,
    "une seule génération doit partir");

  attente.resoudre(ctx.evaluer(`(${JSON.stringify(GENERATION)})`));
  await fini;
});

test("une fenêtre de retouche ne se ferme qu'après le rechargement", async () => {
  // Fermer avant laisse l'écran figé sans rien pour l'expliquer : le récap
  // n'est rechargé qu'ensuite, et ce temps-là n'était couvert par personne.
  const ctx = chargerApp();
  const p = ctx.evaluer("plan");
  p.lignes = ctx.evaluer(`([{ cf: "CF1", participant: "x", raison_sociale: "S", jj: 5,
    pa: "PA", mep_id: 1, mep_date: "2026-09-01", run_num: "3320", run_date: "2026-09-10",
    origine: "auto", etat: "eligible", gelee: false, retire_motif: null }])`);
  p.sel = ctx.evaluer(`(new Set(["CF1"]))`);

  const attente = enAttente();
  ctx.repondreAux((cmd) => (cmd === "plan_lignes" ? attente.p : ctx.evaluer("[]")));

  ctx.app.renderPlanRecap();
  ctx.app.ouvrirRetrait();
  const zone = trouver(ctx.$("modal"), (n) => n.tagName === "textarea");
  zone.value = "compte clôturé";
  zone.listeners.input();   // c'est la saisie qui déverrouille le bouton
  const bouton = trouver(ctx.$("modal"),
    (n) => n.tagName === "button" && libelle(n).startsWith("Retirer "));

  const fini = bouton.click();
  await tick();
  assert.equal(ctx.$("modal-backdrop").classList.contains("hidden"), false,
    "la fenêtre doit rester ouverte tant que le récap n'est pas rechargé");

  attente.resoudre(ctx.evaluer("[]"));
  await fini;
  assert.equal(ctx.$("modal-backdrop").classList.contains("hidden"), true,
    "et se fermer une fois le rechargement terminé");
});

test("un travail qui échappe rétablit quand même le bouton", async () => {
  // Le helper est générique : rien ne garantit que tous ses appelants
  // attraperont leurs erreurs. Aujourd'hui ils le font tous, donc AUCUN
  // parcours d'écran ne peut prouver le `finally` — il se teste ici, sur le
  // helper lui-même, faute de quoi il resterait du code jamais éprouvé.
  const ctx = ecranPlan();
  const b = ctx.$("btn-plan-gen");
  const avant = libelle(b);

  await assert.rejects(
    () => ctx.app.occupe(b, "Génération en cours…", async () => { throw new Error("boum"); }),
    /boum/, "l'erreur doit remonter, pas être avalée par le helper");

  assert.equal(b.disabled, false, "le bouton doit être rétabli malgré l'échappement");
  assert.equal(libelle(b), avant);
  assert.equal(b.dataset.occupe, undefined, "et le verrou levé, sinon plus rien ne repart");
});

// ------------------------------------------------------------ aperçu

test("pendant un recalcul, les chiffres affichés sont marqués périmés", async () => {
  // Le problème n'est pas qu'une tâche tourne, c'est que les chiffres lus ne
  // correspondent plus à ce qu'on vient de saisir.
  const ctx = ecranPlan();
  const attente = enAttente();
  ctx.repondreAux((cmd) => (cmd === "plan_preview" ? attente.p : ctx.evaluer("[]")));

  ctx.app.planRecalc();
  await attendreApercu();

  assert.equal(ctx.$("plan-main").classList.contains("recalcul"), true,
    "la zone des chiffres doit être marquée pendant le calcul");

  attente.resoudre(ctx.evaluer(`(${JSON.stringify(APERCU)})`));
  await tick();
  assert.equal(ctx.$("plan-main").classList.contains("recalcul"), false,
    "et démarquée dès les vrais chiffres arrivés");
});

test("un recalcul en échec ne laisse pas les chiffres grisés à jamais", async () => {
  const ctx = ecranPlan();
  const attente = enAttente();
  ctx.repondreAux((cmd) => (cmd === "plan_preview" ? attente.p : ctx.evaluer("[]")));

  ctx.app.planRecalc();
  await attendreApercu();
  attente.rejeter("rampe géométrique : la raison doit être un nombre strictement positif");
  await tick();

  assert.equal(ctx.$("plan-main").classList.contains("recalcul"), false);
});

test("le panneau de saisie n'est jamais marqué périmé", async () => {
  // C'est la frappe qui déclenche le recalcul : griser ce qu'on est en train
  // de régler serait absurde, et la saisie ne doit pas être interrompue.
  const ctx = ecranPlan();
  const attente = enAttente();
  ctx.repondreAux((cmd) => (cmd === "plan_preview" ? attente.p : ctx.evaluer("[]")));

  ctx.app.planRecalc();
  await attendreApercu();

  assert.equal(ctx.$("plan-main").classList.contains("recalcul"), true, "montage du test");
  assert.equal(ctx.$("plan-aside").classList.contains("recalcul"), false,
    "le panneau de saisie ne doit jamais être atténué");

  attente.resoudre(ctx.evaluer(`(${JSON.stringify(APERCU)})`));
  await tick();
});
