// Ajout de comptes par run : couches d'empilement, tri et filtres.
//
// Vécu en application : la fenêtre d'ajout ouverte depuis le Plan de charge
// était invisible — l'écran plein du plan la recouvrait. Il fallait la fermer
// pour voir ce qu'elle affichait.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const CSS = fs.readFileSync(path.join(__dirname, "..", "src", "styles.css"), "utf8");

/** Le z-index déclaré pour un sélecteur, tel qu'écrit dans la feuille. */
function couche(selecteur) {
  const bloc = CSS.split(selecteur + " {")[1];
  assert.ok(bloc, `sélecteur « ${selecteur} » introuvable dans styles.css`);
  const m = bloc.split("}")[0].match(/z-index:\s*(\d+)/);
  assert.ok(m, `pas de z-index sur « ${selecteur} »`);
  return Number(m[1]);
}

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
