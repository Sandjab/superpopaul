// Saisie au clavier des dates du plan de charge.
//
// Un champ date de navigateur n'a pas de valeur tant qu'il est incomplet —
// sauf l'année, qui vaut dès son premier chiffre. Taper « 2026 » notifie donc
// quatre dates entières : l'an 2, l'an 20, l'an 202, puis 2026. Mesuré dans
// Chromium, moteur de WebView2 sous Windows.
//
// Chacune de ces notifications déclenchait un calcul, sur une fenêtre partant
// de l'an 2 que la timeline parcourt un jour civil à la fois — 739 409 jours.
// Et sur « Ajouter une MEP », elle reconstruisait le panneau : le champ se
// vidait sous les doigts dès le premier chiffre de l'année.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp } = require("./dom_shim");

/** Anti-rebond du recalcul (250 ms) plus une marge, comme plan_attente. */
const attendreApercu = () => new Promise((r) => setTimeout(r, 320));

/** Ce que le navigateur notifie pendant la frappe de « 2026 », dans l'ordre. */
const FRAPPE = ["0002-09-01", "0020-09-01", "0202-09-01", "2026-09-01"];

/** `PlanApercu` minimal — des chiffres déjà à l'écran, qui ne doivent pas
 *  disparaître pendant qu'on retape une date. */
const APERCU = {
  funnel: { lignes: 10, cf_distincts: 10, jj_valide: 10, resolus: 10,
            ctc_ready: 10, ppf_usable: 10, eligibles: 10 },
  timeline: [], stock_jj: [], plateformes: [], avertissements: [],
  meps: ["2026-09-01"], cible: 10, total: 10, geles: 0, epingles: 0, retires: 0,
};

/** Écran de paramétrage rendu, calendrier chargé et fenêtre complète : tout
 *  est réuni pour qu'un aperçu parte. Seule la date saisie fera la différence. */
function ecranPlan() {
  const ctx = chargerApp();
  ctx.evaluer("plan").runs =
    ctx.evaluer(`([{ num: "3320", date: "2026-08-11", jjs: [1], exclu: false }])`);
  ctx.app.renderPlanAside();
  ctx.$("plan-debut").value = "2026-08-01";
  ctx.$("plan-fin").value = "2026-11-30";
  return ctx;
}

const apercus = (ctx) => ctx.invocations.filter(([c]) => c === "plan_preview").length;

test("une année en cours de frappe ne déclenche aucun calcul", async () => {
  const ctx = ecranPlan();

  // Montage : sur une fenêtre complète, l'aperçu part bien. Sans ce témoin, le
  // test passerait aussi si plus RIEN ne déclenchait jamais de calcul.
  ctx.app.planRecalc();
  await attendreApercu();
  assert.equal(apercus(ctx), 1, "montage du test : une fenêtre complète calcule");

  for (const etat of FRAPPE.slice(0, 3)) {
    ctx.$("plan-debut").value = etat;
    ctx.app.planRecalc();
    await attendreApercu();
    assert.equal(apercus(ctx), 1, `l'an ${etat.slice(0, 4)} ne doit rien déclencher`);
  }

  ctx.$("plan-debut").value = FRAPPE[3];
  ctx.app.planRecalc();
  await attendreApercu();
  assert.equal(apercus(ctx), 2, "la date achevée, elle, doit calculer");
});

test("une année en cours de frappe n'efface pas les chiffres affichés", async () => {
  // Vider l'écran à chaque chiffre ferait clignoter le récapitulatif entier.
  // Une frappe n'est pas un effacement : elle ne dit rien de nouveau encore.
  const ctx = ecranPlan();
  ctx.evaluer("plan").apercu = ctx.evaluer(`(${JSON.stringify(APERCU)})`);

  ctx.$("plan-debut").value = "0002-09-01";
  ctx.app.planRecalc();
  await attendreApercu();

  assert.notEqual(ctx.evaluer("plan").apercu, null,
    "les chiffres de la dernière fenêtre complète doivent rester");
});

test("effacer la date, en revanche, efface bien les chiffres", async () => {
  // La garde ne doit pas rendre le champ inerte : un champ vidé est un geste,
  // pas une frappe en cours.
  const ctx = ecranPlan();
  ctx.evaluer("plan").apercu = ctx.evaluer(`(${JSON.stringify(APERCU)})`);

  ctx.$("plan-debut").value = "";
  ctx.app.planRecalc();
  await attendreApercu();

  assert.equal(ctx.evaluer("plan").apercu, null,
    "sans fenêtre, il n'y a plus de chiffres à montrer");
});

test("taper une MEP au clavier n'en ajoute qu'une", () => {
  const ctx = ecranPlan();

  for (const etat of FRAPPE) {
    ctx.$("plan-mepadd").listeners.change({ target: { value: etat } });
  }

  assert.deepEqual(JSON.parse(JSON.stringify(ctx.evaluer("plan").meps)),
    ["2026-09-01"], "les états de frappe ne sont pas des MEP");
});

test("le champ MEP ne se vide pas sous les doigts", () => {
  // Le geste de fin — vider le champ et reconstruire le panneau — détruit le
  // champ en cours de saisie. Il n'appartient qu'à la date achevée.
  const ctx = ecranPlan();
  const champ = { value: "0002-09-01" };

  ctx.$("plan-mepadd").listeners.change({ target: champ });

  assert.equal(champ.value, "0002-09-01",
    "la frappe doit survivre à sa propre notification");
});
