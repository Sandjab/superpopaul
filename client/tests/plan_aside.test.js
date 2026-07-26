// Panneau latéral du Plan de charge : il est reconstruit en entier à chaque
// rendu, et les champs libres n'ont d'état QUE dans le DOM (`planParams` les
// relit sur les éléments). Ces tests tiennent cette contrainte : sans capture
// des valeurs avant reconstruction, le moindre geste rendait la saisie à ses
// défauts — bug vécu en application, seed silencieusement remis à 42 compris.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

// `app.js` tourne dans un contexte `vm`, donc ses tableaux et objets ont les
// prototypes de CE contexte : les comparer en profondeur exige d'en recopier
// la forme ici. Les valeurs scalaires, elles, se comparent directement.
const copie = (x) => JSON.parse(JSON.stringify(x));

/** Panneau rendu une fois, avec une saisie utilisateur complète. */
function panneauRempli() {
  const { app, $ } = chargerApp();
  app.renderPlanAside();
  $("plan-debut").value = "2026-07-01";
  $("plan-fin").value = "2026-12-31";
  $("plan-cible").value = "500";
  $("plan-seed").value = "7";
  $("plan-mepcount").value = "3";
  return { app, $ };
}

const ajouterMep = ($, date) =>
  $("plan-mepadd").listeners.change({ target: { value: date } });

test("ajouter une MEP conserve la saisie du panneau", () => {
  const { app, $ } = panneauRempli();

  ajouterMep($, "2026-08-01");

  const p = app.planParams();
  assert.deepEqual(copie(p.meps), ["2026-08-01"], "la MEP doit bien être ajoutée");
  assert.equal(p.debut, "2026-07-01", "le début de fenêtre FUT ne doit pas être perdu");
  assert.equal(p.fin, "2026-12-31", "la fin de fenêtre FUT ne doit pas être perdue");
  assert.equal(p.cible, 500);
  assert.equal(p.seed, 7, "un seed remis à son défaut changerait le plan sans le dire");
  assert.equal(p.mep_count, 3);
});

test("retirer une MEP conserve la saisie du panneau", () => {
  const { app, $ } = panneauRempli();
  ajouterMep($, "2026-08-01");

  const puce = trouver($("plan-aside"), (n) => n.className === "chip");
  trouver(puce, (n) => n.tagName === "button").listeners.click();

  const p = app.planParams();
  assert.deepEqual(copie(p.meps), [], "la MEP doit bien être retirée");
  assert.equal(p.debut, "2026-07-01");
  assert.equal(p.fin, "2026-12-31");
});

test("la case « pilote » reste cochée après le rendu qu'elle déclenche", () => {
  // Sinon l'écran montre les champs du pilote sous une case décochée, et le
  // rendu suivant les fait disparaître avec ce qui y a été saisi.
  const { app, $ } = panneauRempli();

  const cocher = $("plan-pilote");
  cocher.checked = true;
  cocher.listeners.change();

  assert.equal($("plan-pilote").checked, true);
  $("plan-pilote-runs").value = "4";
  $("plan-pilote-cf").value = "25";

  ajouterMep($, "2026-08-01");
  assert.deepEqual(copie(app.planParams().rampe.pilote), { runs: 4, cf_par_run: 25 });
});

test("changer la forme de rampe conserve la saisie du panneau", () => {
  const { app, $ } = panneauRempli();

  const forme = $("plan-forme");
  forme.value = "geometrique";
  forme.listeners.change();

  const p = app.planParams();
  assert.equal(p.rampe.forme, "geometrique");
  assert.equal(p.debut, "2026-07-01");
  assert.equal(p.seed, 7);
});
