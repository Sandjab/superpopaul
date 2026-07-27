// Ce que l'écran dit du rapport entre le plan enregistré et le fichier ouvert.
//
// L'application notait le nom du fichier ET une empreinte de son contenu, mais
// ne comparait que le nom. Une extraction hebdomadaire qui garde son nom passait
// donc pour le même fichier — alors que les lignes gelées, conservées telles
// quelles à la régénération, décrivent des comptes tels qu'ils étaient.
//
// Quatre états, donc, et quatre messages distincts.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp } = require("./dom_shim");

const PARAMS = {
  runs: [], debut: "2026-08-01", fin: "2026-11-30", meps: [],
  mep_count: 0, cible: null, seed: 42, pa_exclues: [],
  rampe: { forme: "plate", pilote: null },
};

/** Ouvre l'écran avec le rapport au fichier que rend `plan_load`. */
async function ouvrir(rapport) {
  const ctx = chargerApp();
  const enr = { params: PARAMS, fichier: "brm2607.csv", genere_le: 0, rapport };
  ctx.repondreAux((cmd) =>
    cmd === "plan_load" ? ctx.evaluer(`(${JSON.stringify(enr)})`) : ctx.evaluer("[]"));
  await ctx.app.ouvrirPlan();
  return ctx;
}

const pied = (ctx) => ctx.$("plan-foot-info").textContent;
const bandeau = (ctx) => ctx.$("plan-banner").className;

test("même fichier : le pied le nomme, sans alerte", async () => {
  const ctx = await ouvrir("identique");

  assert.equal(pied(ctx), "Plan enregistré depuis brm2607.csv");
  assert.notEqual(bandeau(ctx), "warn", "rien n'a changé : rien à signaler");
});

test("même nom, contenu changé : c'est dit, et c'est le cas qui manquait", async () => {
  const ctx = await ouvrir("contenu_different");

  assert.match(pied(ctx), /contenu a changé/, `pied muet : ${pied(ctx)}`);
  assert.equal(bandeau(ctx), "warn", "le cas dangereux doit lever un avertissement");
  const texte = String(ctx.$("plan-banner").children?.[0] ?? "");
  assert.match(texte, /même nom/, `le message doit dire que le nom, lui, est le même : ${texte}`);
  assert.match(texte, /gelées/, `et pointer ce qui est en cause : ${texte}`);
});

test("autre fichier : le message d'avant, mot pour mot", async () => {
  // Ce cas marchait déjà. Le lot ne doit pas le remuer.
  const ctx = await ouvrir("autre_fichier");

  assert.match(pied(ctx), /le fichier ouvert est différent/);
  assert.equal(bandeau(ctx), "warn");
  const texte = String(ctx.$("plan-banner").children?.[0] ?? "");
  assert.match(texte, /différent du fichier ouvert/, texte);
});

test("fichier illisible : on dit qu'on ne sait pas", async () => {
  // Avant, ce cas produisait « le fichier ouvert est différent » — une
  // affirmation que rien n'étaye. Ne rien dire serait pire : l'absence
  // d'avertissement se lit comme « tout va bien ».
  const ctx = await ouvrir("inconnu");

  assert.match(pied(ctx), /vérification impossible/, `pied : ${pied(ctx)}`);
  assert.notEqual(bandeau(ctx), "warn", "on ne conclut pas, donc on n'alerte pas");
});

test("un état inconnu du frontend ne prétend pas que tout va bien", async () => {
  // Le jour où le moteur en ajoutera un cinquième, l'écran doit se rabattre sur
  // « je ne sais pas », jamais sur le silence.
  const ctx = await ouvrir("un_etat_futur");

  assert.match(pied(ctx), /vérification impossible/, `pied : ${pied(ctx)}`);
});
