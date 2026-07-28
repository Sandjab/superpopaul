// Écran de rapprochement du plan avec le fichier ouvert.
//
// Le cœur métier (rapprochement.rs) est déjà livré et testé côté Rust : ces
// tests ne portent que sur le câblage JS — l'empreinte qui doit voyager
// intacte jusqu'à l'application (sans quoi le backend refuse, pour une raison
// que l'utilisateur ne peut pas deviner), le déclencheur qui reste inerte
// sans écart, et l'affichage d'une erreur backend sans jamais passer par
// innerHTML (comptes, motifs et messages viennent tous d'entrées non fiables).

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

function ligne(cf, extra = {}) {
  return {
    cf, participant: `iso6523-actorid-upis::0225:${cf}`, raison_sociale: `Société ${cf}`,
    jj: 5, pa: "Cegedim", mep_id: 1, mep_date: "2026-09-01", run_num: "RF01",
    run_date: "2026-09-10", origine: "auto", etat: "eligible", gelee: false,
    retire_motif: null, ...extra,
  };
}

const ECART = {
  cf: "CF1", gelee: false,
  nature: { type: "eligibilite_perdue", avant: "CTC prêt", apres: "CTC prêt plus tard" },
  action: { type: "retirer", motif: "Rapprochement du 28/07/2026 — CTC prêt plus tard" },
};

/** Récap prêt pour le rapprochement : un plan déjà généré, une ligne. */
function ecran() {
  const ctx = chargerApp();
  const p = ctx.evaluer("plan");
  p.genere = true;
  p.rapportFichier = "identique";
  p.lignes = ctx.evaluer(`(${JSON.stringify([ligne("CF1")])})`);
  ctx.app.renderPlanRecap();
  return ctx;
}

const boutonRapprocher = ($) => trouver($("plan-recap"),
  (n) => n.tagName === "button" && String(n.children[0] ?? "") === "Rapprocher…");

const boutonModale = ($, debut) => trouver($("modal"),
  (n) => n.tagName === "button" && String(n.children[0] ?? "").startsWith(debut));

test("l'empreinte survit à un re-rendu de l'écran", async () => {
  const ctx = ecran();
  const EMPREINTE = "3f2a9c...empreinte-connue";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [ECART], inchangees: 0, avertissements: [] },
        empreinte: EMPREINTE,
      })})`);
    if (cmd === "plan_lignes") return ctx.evaluer(`(${JSON.stringify([ligne("CF1")])})`);
    if (cmd === "plan_rapprocher_appliquer") return ctx.evaluer("[]");
    return null;
  });

  await boutonRapprocher(ctx.$).click();
  // Un re-rendu du récap (filtre changé, sélection modifiée…) ne doit rien
  // effacer de ce que la revue retient : la modale vit hors de `#plan-recap`,
  // que `renderPlanRecap` reconstruit entièrement.
  ctx.app.renderPlanRecap();

  await boutonModale(ctx.$, "Appliquer").click();

  const appel = ctx.invocations.find(([c]) => c === "plan_rapprocher_appliquer");
  assert.notEqual(appel, undefined, "la commande d'application doit être appelée");
  assert.equal(appel[1].empreinte, EMPREINTE,
    "sans elle, le backend refuse et l'utilisateur lit une erreur incompréhensible");
});

test("sans écart, le déclencheur d'application reste inerte", async () => {
  const ctx = ecran();
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [], inchangees: 2001, avertissements: [] },
        empreinte: "peu importe ici",
      })})`);
    return null;
  });

  await boutonRapprocher(ctx.$).click();

  const appliquer = boutonModale(ctx.$, "Appliquer");
  assert.ok(appliquer, "un bouton Appliquer doit exister, ne serait-ce que désactivé");
  assert.equal(appliquer.disabled, true, "aucun écart : rien à appliquer");
  appliquer.click();
  assert.equal(ctx.invocations.find(([c]) => c === "plan_rapprocher_appliquer"), undefined,
    "sans écart, l'application ne doit jamais partir vers le backend");
});

test("une erreur backend s'affiche telle quelle, jamais via innerHTML", async () => {
  const ctx = ecran();
  const MESSAGE = "le fichier a changé depuis le calcul <script>alert(1)</script>";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher") throw MESSAGE;
    return null;
  });

  await boutonRapprocher(ctx.$).click();

  const banniere = ctx.$("plan-banner");
  assert.equal(banniere.textContent, MESSAGE, "le message doit être posé tel quel, sans interprétation");
  assert.equal(trouver(banniere, (n) => n.tagName === "script"), null,
    "aucun nœud <script> ne doit apparaître dans le DOM : la donnée n'est pas fiable");
});
