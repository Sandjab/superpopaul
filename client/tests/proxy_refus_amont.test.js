// Ce que l'écran des réglages fait d'un refus venu d'un intermédiaire, par
// opposition à un refus du proxy lui-même.
//
// Les deux messages parlent de proxy : celui d'un 407 parce que le proxy réclame
// des identifiants, celui d'un refus d'amont parce qu'il nomme la page de
// confirmation à valider. Un motif qui cherche « proxy » dans le texte confond
// les deux et jette les identifiants déjà saisis — l'utilisateur se les voit
// redemander au clic suivant alors qu'ils n'ont jamais été en cause, et que la
// ressaisie ne débloque rien.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp } = require("./dom_shim");

// Textes rendus par ApiError (api.rs) — c'est leur forme affichée qui est en jeu.
const REFUS_AMONT =
  "La requête n'a pas atteint l'API : un intermédiaire l'a refusée (HTTP 403). "
  + "C'est en général la page de confirmation d'un proxy d'entreprise : ouvrez "
  + "https://api.example dans votre navigateur, validez la page, puis relancez.";
const AUTH_PROXY = "Le proxy demande une authentification (HTTP 407).";

/** App avec un proxy configuré et ses identifiants déjà saisis dans la session,
 *  dont le test de clé échoue avec `erreur`. */
function appDontLeTestEchoue(erreur) {
  const ctx = chargerApp();
  ctx.evaluer(`state.config.api.proxy = { url: "http://proxy:8080" };`);
  ctx.evaluer(`proxyCredsGiven = true; proxyCredsUrl = "http://proxy:8080";`);
  ctx.repondreAux((cmd) => {
    if (cmd === "test_api") throw erreur;
    return null;
  });
  return ctx;
}

test("un refus venu d'un intermédiaire ne jette pas les identifiants proxy", async () => {
  const ctx = appDontLeTestEchoue(REFUS_AMONT);
  await ctx.$("btn-test-api").click();
  assert.equal(
    ctx.evaluer("proxyCredsGiven"),
    true,
    "les identifiants proxy restent valides : ce refus ne vient pas du proxy",
  );
});

test("un refus du proxy lui-même fait redemander les identifiants", async () => {
  const ctx = appDontLeTestEchoue(AUTH_PROXY);
  await ctx.$("btn-test-api").click();
  assert.equal(
    ctx.evaluer("proxyCredsGiven"),
    false,
    "un 407 doit rouvrir la saisie des identifiants au prochain clic",
  );
});

test("le message du refus d'amont s'affiche tel quel", async () => {
  const ctx = appDontLeTestEchoue(REFUS_AMONT);
  await ctx.$("btn-test-api").click();
  const affiche = ctx.$("api-test-result").textContent;
  assert.match(affiche, /navigateur/, `la manœuvre doit rester lisible : ${affiche}`);
});
