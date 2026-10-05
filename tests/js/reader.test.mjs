/**
 * @fileoverview Behavioural tests for the injected markdown reader.
 *
 * Boundary: runs `crates/observe/src/assets/reader.js` -- the real,
 * shipped script -- inside a minimal DOM (`dom.mjs`) and asserts the
 * markdown it produces. The table rules in particular used to be
 * pinned by a string assertion (`!READER_JS.contains("cells.slice(0")`),
 * which a rename would have emptied silently: it could only say the
 * old text was gone, never that the rule still held.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createPage, el, loadScript, makeDocument } from './dom.mjs';

const READER_JS = loadScript('crates/observe/src/assets/reader.js');

/** Reads `body` and returns the envelope. */
function read(body, documentOptions = {}) {
  const page = createPage({
    document: makeDocument({ body, ...documentOptions }).document,
    window: { innerWidth: 800, innerHeight: 600, getComputedStyle: undefined },
  });
  return page.run(READER_JS);
}

const blocks = (markdown) => markdown.split('\n\n');

test('a colspan header does not clip the rows under it', () => {
  // The regression: a markdown table needs one column count for every
  // row, and it has to fit the widest. Clipping the data rows to the
  // header's cell count dropped their trailing cells while the
  // readout still claimed to be complete.
  const markdown = read(
    el('body', {}, [
      el('table', {}, [
        el('tr', {}, [el('th', { attrs: { colspan: 2 } }, ['Group']), el('th', {}, ['Total'])]),
        el('tr', {}, [el('td', {}, ['a']), el('td', {}, ['b']), el('td', {}, ['30'])]),
        el('tr', {}, [el('td', {}, ['c']), el('td', {}, ['d']), el('td', {}, ['40'])]),
      ]),
    ])
  ).markdown;

  assert.deepEqual(blocks(markdown), [
    '| Group | Total |  |',
    '| --- | --- | --- |',
    '| a | b | 30 |',
    '| c | d | 40 |',
  ]);
});

test('a header wider than the rows pads them instead of dropping them', () => {
  const markdown = read(
    el('body', {}, [
      el('table', {}, [
        el('tr', {}, [el('th', {}, ['a']), el('th', {}, ['b']), el('th', {}, ['c'])]),
        el('tr', {}, [el('td', {}, ['only'])]),
      ]),
    ])
  ).markdown;
  assert.deepEqual(blocks(markdown), [
    '| a | b | c |',
    '| --- | --- | --- |',
    '| only |  |  |',
  ]);
});

test('a leading row with no cells does not swallow the table', () => {
  // A spacer or template row used to be taken as the header, so the
  // real header became a data row and the table lost its shape.
  const markdown = read(
    el('body', {}, [
      el('table', {}, [
        el('tr', {}, []),
        el('tr', {}, [el('th', {}, ['name']), el('th', {}, ['size'])]),
        el('tr', {}, [el('td', {}, ['readout']), el('td', {}, ['1 KB'])]),
      ]),
    ])
  ).markdown;
  // The cell-less row is still rendered, as the empty data row it is;
  // what matters is that it no longer became the header.
  assert.deepEqual(blocks(markdown), [
    '| name | size |',
    '| --- | --- |',
    '|  |  |',
    '| readout | 1 KB |',
  ]);
});

test('a table with no cells at all contributes nothing', () => {
  const envelope = read(el('body', {}, [el('table', {}, [el('tr', {}, [])]), el('p', {}, ['after'])]));
  assert.deepEqual(blocks(envelope.markdown), ['after']);
});

test('a nested table contributes its rows to itself, not to the outer one', () => {
  const markdown = read(
    el('body', {}, [
      el('table', {}, [
        el('tr', {}, [el('th', {}, ['outer']), el('td', {}, [el('table', {}, [
          el('tr', {}, [el('th', {}, ['inner']), el('td', {}, ['cell'])]),
        ])])]),
        el('tr', {}, [el('td', {}, ['tail']), el('td', {}, ['more'])]),
      ]),
    ])
  ).markdown;
  assert.deepEqual(blocks(markdown), [
    '| outer | innercell |',
    '| --- | --- |',
    '| tail | more |',
  ]);
});

test('a pipe inside a cell is escaped so the row keeps its shape', () => {
  const markdown = read(
    el('body', {}, [
      el('table', {}, [
        el('tr', {}, [el('th', {}, ['expr'])]),
        el('tr', {}, [el('td', {}, ['a | b'])]),
      ]),
    ])
  ).markdown;
  assert.deepEqual(blocks(markdown), ['| expr |', '| --- |', '| a \\| b |']);
});

test("a page's own backslash cannot re-open an escape", () => {
  // The escapes have to carry the backslash itself. Escaping only the
  // metacharacter leaves the page's own `\` free to consume the escape
  // this adds, so `a\` + `|` would emit `a\\|` -- a markdown parser
  // reads that as an escaped backslash followed by a real column
  // separator, and the row silently gains a column.
  const cell = read(
    el('body', {}, [
      el('table', {}, [
        el('tr', {}, [el('th', {}, ['expr'])]),
        el('tr', {}, [el('td', {}, [String.raw`a \| b`])]),
      ]),
    ])
  ).markdown;
  assert.deepEqual(blocks(cell), ['| expr |', '| --- |', String.raw`| a \\\| b |`]);

  const label = read(
    el('body', {}, [el('p', {}, [el('a', { attrs: { href: '/x' } }, [String.raw`a \] b`])])])
  ).markdown;
  assert.equal(label, String.raw`[a \\\] b](https://page.example/x)`);

  const alt = read(
    el('body', {}, [el('img', { attrs: { src: 'pic.png', alt: String.raw`a \] b` } }, [])])
  ).markdown;
  assert.equal(alt, String.raw`![a \\\] b](https://page.example/a/pic.png)`);
});

test('a page past the character budget is marked truncated and clipped', () => {
  // Without the flag a clipped readout presented cut output as
  // complete, and the page-side budget is the only place the reader
  // can own up to it before the converter's own clamp runs.
  const paragraph = 'x'.repeat(19_999);
  const envelope = read(
    el('body', {}, Array.from({ length: 8 }, () => el('p', {}, [paragraph])))
  );
  assert.equal(envelope.truncated, true, 'an over-budget readout says so');
  assert.equal(
    envelope.markdown.length,
    100_000,
    'the markdown stops at the character cap'
  );
  assert.ok(envelope.markdown.endsWith('...'), 'the cut is visible, not silent');
});

test('a page inside the budget is not marked truncated', () => {
  const envelope = read(el('body', {}, [el('p', {}, ['short enough'])]));
  assert.equal(envelope.truncated, false);
  assert.equal(envelope.markdown, 'short enough');
});

test('site chrome and interaction surfaces are omitted', () => {
  const envelope = read(
    el('body', {}, [
      el('nav', {}, ['navigation']),
      el('footer', {}, ['footer text']),
      el('div', { attrs: { role: 'complementary' } }, ['sidebar']),
      el('div', { attrs: { 'aria-hidden': 'true' } }, ['decorative']),
      el('button', {}, ['Click me']),
      el('input', {}, []),
      el('p', { rects: false }, ['offscreen']),
      el('p', {}, ['the content']),
    ])
  );
  assert.deepEqual(blocks(envelope.markdown), ['the content']);
});

test('a role borrowed from Object.prototype is not site chrome', () => {
  // Chrome is matched by tag or landmark role, and the role string is
  // the page's own. An object-literal table also answers for
  // `constructor` and the rest of `Object.prototype`, so a page naming
  // one of those would have its content taken for chrome and dropped
  // from the readout.
  const envelope = read(
    el('body', {}, [el('div', { attrs: { role: 'constructor' } }, ['the content'])])
  );
  assert.deepEqual(blocks(envelope.markdown), ['the content']);
});

test('a relative link is resolved against the document', () => {
  const markdown = read(
    el('body', {}, [el('p', {}, ['see ', el('a', { attrs: { href: '../docs' } }, ['the docs'])])])
  ).markdown;
  assert.equal(markdown, 'see [the docs](https://page.example/docs)');
});

test('a heading keeps its level and emphasis survives outside a paragraph', () => {
  const markdown = read(
    el('body', {}, [
      el('h3', {}, ['Title']),
      el('div', {}, ['Hello ', el('strong', {}, ['world']), ' again']),
    ])
  ).markdown;
  assert.deepEqual(blocks(markdown), ['### Title', 'Hello **world** again']);
});

test('a list indents its nested lists', () => {
  const markdown = read(
    el('body', {}, [
      el('ul', {}, [
        el('li', {}, ['outer', el('ul', {}, [el('li', {}, ['inner'])])]),
        el('li', {}, ['second']),
      ]),
    ])
  ).markdown;
  assert.deepEqual(blocks(markdown), ['- outer', '  - inner', '- second']);
});

test('a blockquote prefixes every line its content produced', () => {
  const markdown = read(
    el('body', {}, [
      el('blockquote', {}, [el('p', {}, ['quoted']), el('pre', {}, ['code()'])]),
    ])
  ).markdown;
  assert.deepEqual(blocks(markdown), ['> quoted', '> ```\n> code()\n> ```']);
});

test('a code fence grows past the longest run inside it', () => {
  const markdown = read(el('body', {}, [el('pre', {}, ['a ``` b'])])).markdown;
  assert.equal(markdown, '````\na ``` b\n````');
});

test('an image becomes a markdown image with an absolute source', () => {
  const markdown = read(
    el('body', {}, [el('img', { attrs: { src: 'pic.png', alt: 'a picture' } }, [])])
  ).markdown;
  assert.equal(markdown, '![a picture](https://page.example/a/pic.png)');
});

test('the title is clipped and reported beside the markdown', () => {
  const envelope = read(el('p', {}, ['body text']), { title: 'T'.repeat(500) });
  assert.equal(envelope.title.length, 200);
  assert.ok(envelope.title.endsWith('...'));
  assert.equal(envelope.version, 1);
});
