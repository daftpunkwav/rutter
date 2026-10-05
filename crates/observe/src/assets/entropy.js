/**
 * @fileoverview Document-start entropy capture for rutter ref scopes.
 *
 * A document's ref scope is what keeps a stale reference captured on
 * one page from resolving to another page's element after a tab switch
 * (see `serializer.js`). The serializer mints that scope from
 * `crypto.getRandomValues`, and `crypto` is an ordinary global a page
 * can replace: a page that fills every buffer with the same bytes would
 * hand each document it opens the same scope, which is the collision
 * the scope exists to prevent.
 *
 * The engine installs this script with
 * `Page.addScriptToEvaluateOnNewDocument`, so it runs before any script
 * the document itself carries. It binds the native generator once and
 * locks it onto the global as a non-configurable, non-writable
 * property: a page script can then replace neither the function nor the
 * property that carries it. A document that predates the installation
 * has no such property, and the serializer falls back to `crypto`
 * there.
 *
 * Never throws. A document with no `crypto` gets no capture at all, and
 * the serializer's store degrades. One whose `crypto` throws is
 * captured like any other, because binding a function does not call it:
 * the throw lands where the serializer calls the generator, which is
 * inside the guard that degrades the store.
 *
 * Owned by rutter-observe; engine code injects it and must not modify
 * it. ASCII-only, plain ES, no build step.
 *
 * No `use strict` directive, unlike the other assets: the engine
 * injects this file as a classic script at document start, and an
 * analyzer reading `.js` as a module reports the directive as
 * redundant. The body is a bind, a property definition, and a
 * try/catch, with no sloppy-mode hazard either way.
 */
(() => {
  try {
    const native = crypto.getRandomValues.bind(crypto);
    Object.defineProperty(window, '__rutterGetRandomValues', {
      value: (bytes) => native(bytes),
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch {
    // No usable `crypto` in this document: there is nothing to capture,
    // and the serializer's fallback covers it.
  }
})();
