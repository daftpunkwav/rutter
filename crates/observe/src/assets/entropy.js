/**
 * @fileoverview Document-start ref-scope minter for rutter.
 *
 * A document's ref scope is what keeps a stale reference captured on one
 * page from resolving to another page's element after a tab switch (see
 * `serializer.js`). The scope therefore has to be a value the document
 * cannot choose, and every global a page can reach is one it can
 * replace: `Math.random`, `crypto.getRandomValues`, `Uint8Array`, and
 * the prototypes the digits would be read through are all writable from
 * page script.
 *
 * The engine installs this script with
 * `Page.addScriptToEvaluateOnNewDocument`, twice: once for the main
 * world, which defines the minter the serializer reads, and once for a
 * named isolated world, where no page script can reach. The shadow copy
 * never runs in the serializer; its presence in that world is the proof
 * that the document-start registration covered the document that is
 * current, so the main-world property was installed by the engine and
 * not by the page. Everything the minter needs is captured here, while
 * the platform is still the platform: the generator is bound, the
 * constructor and the digit table are held as values, and the conversion
 * uses only operators and indexing, which no page can replace. The
 * minter itself is locked onto the global as a non-configurable,
 * non-writable property.
 *
 * A document that predates the installation -- a target that was already
 * loaded when rutter attached -- is covered by the engine minting a
 * scope for it in an isolated world and defining the same property there.
 * A document with neither has no minter, and the serializer hands out no
 * references rather than a scope the page could have chosen.
 *
 * Never throws. A document with no `crypto` gets no minter at all, and
 * one whose `crypto` throws is captured like any other, because binding
 * a function does not call it: the throw lands where the serializer
 * calls the minter.
 *
 * Owned by rutter-observe; engine code injects it and must not modify
 * it. ASCII-only, plain ES, no build step.
 *
 * No `use strict` directive, unlike the other assets: the engine injects
 * this file as a classic script at document start, and an analyzer
 * reading `.js` as a module reports the directive as redundant. The body
 * is a bind, a property definition, and a try/catch, with no
 * sloppy-mode hazard either way.
 */
(() => {
  try {
    const fill = crypto.getRandomValues.bind(crypto);
    const Bytes = Uint8Array;
    const fromCharCode = String.fromCharCode;
    const WIDTH = 4;

    Object.defineProperty(window, '__rutterRefScope', {
      value: () => {
        const bytes = new Bytes(WIDTH);
        fill(bytes);
        // Read and converted without a computed property access and
        // without a prototype method: the four bytes are read by their
        // literal positions, and the digits come from arithmetic on
        // character codes through a `fromCharCode` captured above. A page
        // can replace neither.
        const value =
          bytes[0] * 16777216 + bytes[1] * 65536 + bytes[2] * 256 + bytes[3];
        let scope = '';
        let rest = value;
        for (let index = 0; index < WIDTH; index += 1) {
          const digit = rest % 36;
          // 0-9 are 48-57 and a-z are 97-122, so 10-35 land on a-z.
          scope = fromCharCode(digit < 10 ? 48 + digit : 87 + digit) + scope;
          rest = (rest - digit) / 36;
        }
        return scope;
      },
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch {
    // No usable `crypto` in this document: there is nothing to capture,
    // and the serializer's store degrades instead of minting from a
    // source the page could replace.
  }
})();
