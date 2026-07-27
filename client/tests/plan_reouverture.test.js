// Fermer l'écran Plan de charge et y revenir.
//
// Fermer ne fait que masquer l'écran : tout l'état vit encore en mémoire.
// Mais rouvrir relisait le plan enregistré et l'imposait — une cible modifiée
// sans régénérer revenait à sa valeur d'avant, en silence, et l'onglet actif
// retombait sur « Paramètres ».
//
// La règle voulue : le plan enregistré est relu UNE fois par session. Rouvrir
// l'écran ne perd rien ; c'est le redémarrage de l'application qui fait
// repartir de la dernière génération.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp } = require("./dom_shim");

/** Ce que rend `plan_load` quand une génération existe en base. */
const ENREGISTRE = {
  params: {
    runs: [{ num: "3320", date: "2026-08-11", jjs: [1], exclu: false }],
    debut: "2026-08-01", fin: "2026-11-30", meps: ["2026-09-01"],
    mep_count: 1, cible: 500, seed: 7, pa_exclues: [],
    rampe: { forme: "plate", pilote: null },
  },
  fichier: "comptes.csv", genere_le: 0, rapport: "identique",
};

/** Contexte dont le backend rend un plan enregistré, et rien d'autre. */
function ecran() {
  const ctx = chargerApp();
  ctx.repondreAux((cmd) =>
    cmd === "plan_load" ? ctx.evaluer(`(${JSON.stringify(ENREGISTRE)})`) : ctx.evaluer("[]"));
  return ctx;
}

const chargements = (ctx) => ctx.invocations.filter(([c]) => c === "plan_load").length;

test("la première ouverture restaure le plan enregistré", async () => {
  // Témoin : sans lui, les tests suivants passeraient aussi si plus rien
  // n'était jamais restauré.
  const ctx = ecran();

  await ctx.app.ouvrirPlan();

  assert.equal(ctx.$("plan-cible").value, "500", "la cible enregistrée doit revenir");
  assert.equal(ctx.$("plan-debut").value, "2026-08-01");
  assert.equal(chargements(ctx), 1);
});

test("rouvrir l'écran ne relit pas le plan enregistré", async () => {
  const ctx = ecran();
  await ctx.app.ouvrirPlan();

  // Une saisie que l'utilisateur n'a pas encore générée.
  ctx.$("plan-cible").value = "800";
  ctx.app.fermerPlan();
  await ctx.app.ouvrirPlan();

  assert.equal(ctx.$("plan-cible").value, "800",
    "une saisie non générée ne doit pas être écrasée par le plan enregistré");
  assert.equal(chargements(ctx), 1,
    "le plan enregistré ne se relit qu'une fois par session");
});

test("rouvrir l'écran conserve l'onglet où on était", async () => {
  const ctx = ecran();
  await ctx.app.ouvrirPlan();

  ctx.app.planShowTab("recap");
  ctx.app.fermerPlan();
  await ctx.app.ouvrirPlan();

  assert.equal(ctx.evaluer("plan").tab, "recap", "l'onglet actif doit être conservé");
  assert.equal(ctx.$("plan-recap").classList.contains("hidden"), false,
    "et le récapitulatif rester à l'écran");
});

test("rouvrir l'écran conserve le calendrier chargé dans la session", async () => {
  // Le cas où RIEN n'est enregistré : le calendrier importé n'existe qu'en
  // mémoire, et une relecture le remettrait à vide.
  const ctx = chargerApp();
  ctx.repondreAux(() => ctx.evaluer("[]"));   // plan_load rend null
  await ctx.app.ouvrirPlan();

  ctx.evaluer("plan").runs =
    ctx.evaluer(`([{ num: "3320", date: "2026-08-11", jjs: [1], exclu: false }])`);
  ctx.app.fermerPlan();
  await ctx.app.ouvrirPlan();

  assert.equal(ctx.evaluer("plan").runs.length, 1,
    "le calendrier importé ne doit pas disparaître en refermant l'écran");
});
