// Câblage de la loupe : ce que la modale demande au backend, et quand.
//
// Le piège visé : un champ vide qui part quand même sur le réseau, et un échec
// réseau qui masquerait les annuaires locaux — ils répondent sans réseau, les
// cacher priverait l'utilisateur d'informations que la machine possède.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp } = require("./dom_shim");

const REPONSE = {
  saisi: "552100554",
  canonique: "iso6523-actorid-upis::0225:552100554",
  mode: "api",
  reseau: { etat: "echec", message: "La requête n'a pas atteint l'API (HTTP 403)." },
  annuaire_peppol: { etat: "repond", in_directory: true },
  ppf: { etat: "muette", raison: "annuaire_vide" },
};

function ouvrir(reponse = REPONSE) {
  const ctx = chargerApp();
  const appels = [];
  ctx.repondreAux((cmd, args) => {
    appels.push([cmd, args]);
    return cmd === "resoudre_adressage" ? ctx.evaluer(`(${JSON.stringify(reponse)})`) : null;
  });
  ctx.$("btn-resolve").click();
  return { ctx, appels };
}

test("une saisie vide ne part pas sur le réseau", async () => {
  const { ctx, appels } = ouvrir();
  await ctx.$("resolve-go").click();
  assert.equal(
    appels.filter(([c]) => c === "resoudre_adressage").length,
    0,
    "aucun appel ne doit partir sans adressage",
  );
});

test("l'adressage saisi est transmis tel quel au backend", async () => {
  const { ctx, appels } = ouvrir();
  ctx.$("resolve-input").value = "552100554";
  await ctx.$("resolve-go").click();
  const appel = appels.find(([c]) => c === "resoudre_adressage");
  assert.ok(appel, "la commande doit être appelée");
  assert.equal(appel[1].saisi, "552100554");
});

test("un échec réseau n'empêche pas d'afficher les annuaires", async () => {
  const { ctx } = ouvrir();
  ctx.$("resolve-input").value = "552100554";
  await ctx.$("resolve-go").click();
  const txt = ctx.$("resolve-result").textContent;
  assert.match(txt, /HTTP 403/, `le message d'échec doit être lisible : ${txt}`);
  assert.match(txt, /in_directory/, `l'annuaire Peppol doit rester affiché : ${txt}`);
});

test("une source muette ne s'affiche jamais « false »", async () => {
  const { ctx } = ouvrir();
  ctx.$("resolve-input").value = "552100554";
  await ctx.$("resolve-go").click();
  const txt = ctx.$("resolve-result").textContent;
  assert.match(txt, /annuaire vide/, `la raison du silence doit être dite : ${txt}`);
  assert.doesNotMatch(
    txt, /ppf_usable\s*false/,
    `« je ne sais pas » ne doit pas se lire « non » : ${txt}`,
  );
});
