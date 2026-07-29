// Câblage de la loupe : ce que la modale demande au backend, et quand.
//
// Le piège visé : un champ vide qui part quand même sur le réseau, et un échec
// réseau qui masquerait les annuaires locaux — ils répondent sans réseau, les
// cacher priverait l'utilisateur d'informations que la machine possède.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

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

test("le bouton ne s'offre qu'une fois la saisie remplie", async () => {
  const { ctx } = ouvrir();
  const go = ctx.$("resolve-go");
  assert.equal(go.disabled, true, "à l'ouverture le champ est vide : rien à résoudre");
  const champ = ctx.$("resolve-input");
  champ.value = "552100554";
  champ.listeners.input({ target: champ }); // c'est la saisie qui déverrouille
  assert.equal(go.disabled, false, "une saisie non vide doit rouvrir le bouton");
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

test("une note du résolveur paraît entière, hors de l'en-tête et sans chasser la latence", async () => {
  // Une note porte une URL : dans `.resolve-sect-h` (capitales espacées,
  // `overflow: hidden`) elle serait criée puis rognée sans que rien ne le dise,
  // alors que c'est elle qui explique pourquoi un compte n'est pas éligible.
  const NOTE = "ServiceGroup HTTP 403 on https://B-abc.iso6523-actorid-upis.edelivery.tech.europa.eu/x";
  const { ctx } = ouvrir({
    ...REPONSE,
    reseau: {
      etat: "repond", latence_ms: 87,
      champs: { in_peppol: true, ctc_status: "ready", note: NOTE },
    },
  });
  ctx.$("resolve-input").value = "552100554";
  await ctx.$("resolve-go").click();
  const entete = trouver(ctx.$("resolve-result"), (n) => n.className === "resolve-sect-h");
  assert.match(entete.textContent, /87 ms/, "une note ne remplace pas la latence, elle s'y ajoute");
  assert.doesNotMatch(entete.textContent, /HTTP 403/, "la note n'a rien à faire dans l'en-tête");
  const note = trouver(ctx.$("resolve-result"), (n) => n.className === "resolve-note");
  assert.ok(note, "la note doit avoir sa propre ligne");
  assert.equal(note.textContent, NOTE, "et être rendue entière, URL comprise");
});

test("une source muette ne s'affiche jamais « false »", async () => {
  const { ctx } = ouvrir();
  ctx.$("resolve-input").value = "552100554";
  await ctx.$("resolve-go").click();
  // L'assertion porte sur la SEULE section PPF, pas sur le résultat entier :
  // le textContent d'une ligne de vedette intercale le libellé humain entre le
  // nom et la valeur (« ppf_usablePPF utilisablefalse »), si bien qu'y chercher
  // le couple nom-valeur ne peut jamais matcher. Section isolée, la règle
  // s'énonce simplement : une source qui se tait n'écrit aucun « false ».
  const ppf = trouver(ctx.$("resolve-result"),
    (n) => n.className === "resolve-sect" && n.textContent.includes("Annuaire PPF"));
  assert.ok(ppf, "la section PPF doit paraître même quand la source se tait");
  const txt = ppf.textContent;
  assert.match(txt, /annuaire vide/, `la raison du silence doit être dite : ${txt}`);
  assert.doesNotMatch(txt, /false/, `« je ne sais pas » ne doit pas se lire « non » : ${txt}`);
});
