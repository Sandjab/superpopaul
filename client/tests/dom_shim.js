// Faux DOM minimal pour exécuter le VRAI `src/app.js` hors navigateur.
//
// Le frontend est en vanilla sans bundler : il n'y a pas de runner à hériter,
// et les bugs qu'on y trouve sont des bugs de câblage (un champ reconstruit
// qui perd sa valeur, un écouteur qui ne rebranche pas). Ce shim ne modélise
// donc que ce dont `app.js` se sert, mais il est FIDÈLE sur le point qui les
// révèle : un élément neuf n'a que la valeur portée par son attribut, la
// saisie de l'utilisateur ne le suit pas.
//
// Volontairement hors de portée : layout, styles, événements qui bouillonnent,
// sélecteurs CSS. Un test qui en aurait besoin doit passer par l'application.

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const CHEMIN_APP = path.join(__dirname, "..", "src", "app.js");

function creerDocument() {
  // `getElementById` rend le DERNIER élément créé sous cet id, comme le vrai
  // DOM rend celui qui est attaché après un remplacement de sous-arbre.
  const parId = new Map();

  function createElement(tag) {
    const el = {
      tagName: tag,
      className: "",
      attrs: {},
      listeners: {},
      children: [],
      textContent: "",
      style: {},
      dataset: {},
      classList: { add() {}, remove() {}, toggle() {}, contains: () => false },
      setAttribute(k, v) {
        this.attrs[k] = String(v);
        if (k === "id") parId.set(String(v), this);
      },
      getAttribute(k) { return this.attrs[k]; },
      removeAttribute(k) { delete this.attrs[k]; },
      addEventListener(evt, fn) { this.listeners[evt] = fn; },
      append(...kids) { for (const k of kids) if (k != null) this.children.push(k); },
      replaceChildren(...kids) { this.children = []; this.append(...kids); },
      querySelectorAll: () => [],
      focus() {}, click() {}, remove() {},
    };
    Object.defineProperty(el, "value", {
      get() {
        if (this._value !== undefined) return this._value;
        // Un <select> tire sa valeur de son option sélectionnée, la première
        // à défaut — c'est ainsi qu'`app.js` restaure la forme de rampe.
        if (this.tagName === "select") {
          const opt = this.children.find((c) => c.selected) ?? this.children[0];
          return opt ? (opt.attrs.value ?? "") : "";
        }
        return this.attrs.value ?? "";
      },
      set(v) { this._value = String(v); },
    });
    // `setAttribute("checked", false)` COCHE la case dans un vrai navigateur :
    // l'attribut compte par sa présence. Le shim reproduit ce piège, c'est lui
    // qui a fait passer la case « pilote » pour décochée.
    Object.defineProperty(el, "checked", {
      get() { return this._checked !== undefined ? this._checked : "checked" in this.attrs; },
      set(v) { this._checked = !!v; },
    });
    return el;
  }

  // Un id inconnu rend un élément factice : `app.js` câble au chargement des
  // écouteurs sur des ids qui vivent dans index.html, absent ici.
  function getElementById(id) {
    if (!parId.has(id)) {
      const el = createElement("div");
      el.setAttribute("id", id);
    }
    return parId.get(id);
  }

  return {
    createElement,
    getElementById,
    querySelectorAll: () => [],
    addEventListener() {},
    body: createElement("body"),
  };
}

/** Charge `src/app.js` dans un contexte NEUF (aucun état partagé entre tests).
 *
 *  Rend :
 *  - `app`  : les fonctions déclarées par app.js (`renderPlanAside`, `planParams`…) ;
 *  - `$`    : `getElementById` du faux document ;
 *  - `invocations` : les `invoke("commande", args)` partis vers le backend. */
function chargerApp() {
  const document = creerDocument();
  const invocations = [];
  const invoke = (cmd, args) => { invocations.push([cmd, args]); return Promise.resolve(null); };
  const window = {
    __TAURI__: {
      core: { invoke },
      event: { listen: () => Promise.resolve(() => {}) },
      dialog: { open: async () => null, save: async () => null },
    },
    addEventListener() {},
    document,
  };

  // Les minuteries d'`app.js` (anti-rebond du recalcul) ne doivent pas retenir
  // le process de test : `unref` les laisse fonctionner sans le maintenir en vie.
  const minuterie = (pose) => (fn, ms) => { const t = pose(fn, ms); t.unref?.(); return t; };

  const ctx = vm.createContext({
    document, window, console,
    setTimeout: minuterie(setTimeout), clearTimeout,
    setInterval: minuterie(setInterval), clearInterval,
    Promise, JSON, Math, Date, Intl, Number, String, Array, Object, Set, Map, Error,
    TextEncoder, TextDecoder, structuredClone,
  });
  ctx.globalThis = ctx;
  vm.runInContext(fs.readFileSync(CHEMIN_APP, "utf8"), ctx, { filename: "app.js" });

  return { app: ctx, $: document.getElementById, invocations };
}

/** Premier nœud du sous-arbre satisfaisant `predicat`, en profondeur d'abord. */
function trouver(noeud, predicat) {
  if (predicat(noeud)) return noeud;
  for (const enfant of noeud.children ?? []) {
    const t = trouver(enfant, predicat);
    if (t) return t;
  }
  return null;
}

module.exports = { chargerApp, trouver };
