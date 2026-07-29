// Les infobulles de la loupe recopient docs/legende_champs.md. Ce test est le
// seul lien mécanique entre les deux : sans lui, corriger la légende laisserait
// l'application afficher l'ancienne définition, indéfiniment.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { chargerApp } = require("./dom_shim");

const MD = fs.readFileSync(
  path.join(__dirname, "..", "..", "docs", "legende_champs.md"), "utf8");

/** Description du champ dans le tableau markdown : | `nom` | libellé | description | … */
function ligneLegende(nom) {
  const re = new RegExp(`^\\|\\s*\`${nom}\`\\s*\\|([^|]*)\\|([^|]*)\\|`, "m");
  const m = MD.match(re);
  return m && { libelle: m[1].trim(), description: m[2].trim() };
}

test("chaque infobulle reprend le libellé et la description de la légende", () => {
  const ctx = chargerApp();
  const legende = ctx.evaluer("LEGENDE");
  const noms = Object.keys(legende);
  assert.ok(noms.length >= 13, `13 champs attendus, ${noms.length} trouvés`);

  for (const nom of noms) {
    const source = ligneLegende(nom);
    assert.ok(source, `${nom} est absent de docs/legende_champs.md`);
    // Le markdown porte des **gras** et des `codes` que l'infobulle aplatit :
    // on compare le texte nu.
    const nu = (s) => s.replace(/[*`]/g, "").replace(/\s+/g, " ").trim();
    assert.equal(
      nu(legende[nom]),
      nu(`${source.libelle} — ${source.description}`),
      `l'infobulle de ${nom} a divergé de docs/legende_champs.md`,
    );
  }
});
