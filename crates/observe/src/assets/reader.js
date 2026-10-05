/**
 * @fileoverview In-page reader for rutter markdown readouts.
 *
 * Walks the composed DOM (including open shadow roots and slot
 * assignments) and extracts the readable content as a Markdown
 * document, returned as a JSON envelope
 * `{ version, truncated, title, markdown }`. Site chrome (nav, aside,
 * footer, landmark roles), hidden elements, and non-content tags are
 * omitted. The script never throws: guards degrade to skipped content
 * or `truncated: true` with partial output, never an exception.
 *
 * The caller evaluates this script inside a page and converts the
 * returned envelope with rutter-observe. Owned by rutter-observe;
 * engine and orchestration code must not modify it.
 */
(() => {
  'use strict';

  const VERSION = 1;
  const MAX_NODES = 50000;
  const MAX_DEPTH = 200;
  // Counted in UTF-16 code units (String.prototype.length): astral-plane
  // characters (emoji, CJK extensions) cost two units, so this guard
  // stops somewhat earlier than 100 000 characters on such pages. The
  // converter's own clamp is character-counted and authoritative.
  const MAX_CHARS = 100000;
  const MAX_INLINE_CHARS = 20000;
  const TITLE_LIMIT = 200;

  // The lookup tables below are Sets, never object literals. Their keys
  // come from the page -- a tag name, an author-supplied `role` -- and
  // an object literal also answers for `constructor`, `toString`, and
  // the rest of `Object.prototype`: a page naming one of those as its
  // role would have its content taken for site chrome. A Set holds
  // exactly what was put in it.

  // Non-content elements: never rendered. Form controls and dialogs
  // are interaction surfaces the accessibility snapshot already
  // covers; the readout is for readable content.
  const SKIPPED_TAGS = new Set([
    'SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD',
    'META', 'LINK', 'TITLE', 'BASE', 'DATALIST',
    'SVG', 'IFRAME', 'CANVAS', 'DIALOG',
    'INPUT', 'TEXTAREA', 'SELECT', 'OPTION', 'BUTTON'
  ]);

  // Site chrome: content that frames the page rather than belonging
  // to it, by tag or landmark role.
  const CHROME_TAGS = new Set(['NAV', 'ASIDE', 'FOOTER']);
  const CHROME_ROLES = new Set([
    'navigation', 'complementary', 'banner', 'contentinfo'
  ]);

  const state = { nodes: 0, size: 0, truncated: false, blocks: [] };

  function clip(text, limit) {
    if (text.length <= limit) return text;
    let sliced = text.slice(0, limit - 3);
    // Never cut between a surrogate pair: a lone surrogate breaks
    // JSON consumers. Drop the orphaned high surrogate if present.
    const last = sliced.charCodeAt(sliced.length - 1);
    if (last >= 0xd800 && last <= 0xdbff) sliced = sliced.slice(0, -1);
    return sliced + '...';
  }

  function collapse(text) {
    return String(text || '').replace(/\s+/g, ' ').trim();
  }

  function isVisible(el) {
    try {
      const rects = el.getClientRects();
      if (!rects || rects.length === 0) return false;
      const style = window.getComputedStyle(el);
      return style.visibility !== 'hidden' && style.visibility !== 'collapse';
    } catch {
      return false;
    }
  }

  function outOfBudget() {
    return state.nodes >= MAX_NODES || state.size >= MAX_CHARS;
  }

  function attribute(el, name) {
    try {
      return el.getAttribute(name);
    } catch {
      return null;
    }
  }

  function absoluteUrl(url) {
    try {
      return new URL(url, document.baseURI).href;
    } catch {
      return null;
    }
  }

  function tagName(el) {
    try {
      return el.tagName;
    } catch {
      return '';
    }
  }

  // `children` and `rows` are HTMLCollections: array-like and live, but
  // not iterable (only NodeList declares `iterable<Node>`), so a walker
  // copies one before reading it. The copy also freezes the walk
  // against a page that mutates the tree mid-readout.
  function arrayOf(collection) {
    return Array.prototype.slice.call(collection || []);
  }

  function isChrome(el) {
    if (CHROME_TAGS.has(tagName(el))) return true;
    const role = attribute(el, 'role');
    if (role) {
      const first = String(role).trim().split(/\s+/)[0];
      if (CHROME_ROLES.has(first)) return true;
    }
    return attribute(el, 'aria-hidden') === 'true';
  }

  function shouldSkip(el) {
    if (SKIPPED_TAGS.has(tagName(el))) return true;
    if (isChrome(el)) return true;
    return !isVisible(el);
  }

  // Inline content of `el` as markdown: text plus links, images, and
  // lightweight emphasis. Capped per call so one huge paragraph
  // cannot balloon the output; the cap sets `truncated`.
  function inlineOf(el) {
    const out = inlineNodes(el.childNodes, 0);
    return collapse(out);
  }

  function inlineNodes(list, depth) {
    let out = '';
    for (const node of list) {
      if (out.length >= MAX_INLINE_CHARS) {
        state.truncated = true;
        break;
      }
      if (!node) continue;
      if (node.nodeType === 3) {
        out += String(node.nodeValue || '').replace(/\s+/g, ' ');
        continue;
      }
      if (node.nodeType !== 1 || depth >= MAX_DEPTH) continue;
      const tag = tagName(node);
      if (SKIPPED_TAGS.has(tag) || isChrome(node) || !isVisible(node)) continue;
      if (tag === 'BR') {
        out += ' ';
        continue;
      }
      if (tag === 'A' && attribute(node, 'href')) {
        const inner = collapse(inlineNodes(node.childNodes, depth + 1));
        if (!inner) continue;
        const href = absoluteUrl(attribute(node, 'href'));
        const label = inner.replace(/\[/g, '\\[').replace(/\]/g, '\\]');
        out += href ? '[' + label + '](' + href + ')' : label;
        continue;
      }
      if (tag === 'IMG') {
        const src = absoluteUrl(attribute(node, 'src') || '');
        if (src) {
          const alt = collapse(attribute(node, 'alt') || '');
          out += '![' + alt.replace(/\]/g, '\\]') + '](' + src + ')';
        }
        continue;
      }
      if (tag === 'CODE' || tag === 'KBD' || tag === 'SAMP') {
        const code = collapse(node.textContent);
        out += code.indexOf('`') === -1 ? '`' + code + '`' : code;
        continue;
      }
      if (tag === 'STRONG' || tag === 'B') {
        out += '**' + collapse(inlineNodes(node.childNodes, depth + 1)) + '**';
        continue;
      }
      if (tag === 'EM' || tag === 'I') {
        out += '*' + collapse(inlineNodes(node.childNodes, depth + 1)) + '*';
        continue;
      }
      out += inlineNodes(node.childNodes, depth + 1);
    }
    return out;
  }

  function push(text) {
    if (!text) return;
    state.blocks.push(text);
    state.size += text.length;
    if (state.size >= MAX_CHARS) state.truncated = true;
  }

  function pushParagraph(text) {
    const collapsed = collapse(text);
    if (collapsed) push(collapsed);
  }

  // Renders `render` and prefixes every line it produced, so
  // blockquotes can wrap any inner shape (paragraphs, pre, tables).
  function prefixed(prefix, render) {
    const start = state.blocks.length;
    render();
    const rendered = state.blocks.slice(start).map((block) =>
      block
        .split('\n')
        .map((line) => (line ? prefix + line : prefix.trimEnd()))
        .join('\n'));
    state.blocks = state.blocks.slice(0, start).concat(rendered);
  }

  function escapeCell(text) {
    return collapse(text).replace(/\|/g, '\\|');
  }

  function headingLevel(el) {
    const tag = tagName(el);
    const level = parseInt(tag.charAt(1), 10);
    if (!(level >= 1 && level <= 6)) return 2;
    return level;
  }

  // The fence has to outrun the longest run of backticks in the text,
  // so one scan counts them. Scanned rather than matched with a regex
  // literal carrying a backtick: that construct derails the JavaScript
  // lexer of the complexity analyzer this repository runs, which then
  // attributes the rest of the file to this one function.
  function fenceFor(text) {
    let longest = 0;
    let run = 0;
    for (const ch of text) {
      run = ch === '`' ? run + 1 : 0;
      if (run > longest) longest = run;
    }
    return '`'.repeat(Math.max(3, longest + 1));
  }

  function emitCode(el) {
    const text = String(el.textContent || '').replace(/\n{3,}/g, '\n\n');
    const fence = fenceFor(text);
    push(fence + '\n' + text.replace(/\n+$/, '') + '\n' + fence);
  }

  function emitList(el, indent, depth) {
    const ordered = tagName(el) === 'OL';
    let index = 1;
    for (const li of arrayOf(el.children)) {
      if (state.truncated || outOfBudget()) return;
      if (!li || tagName(li) !== 'LI') continue;
      if (!isVisible(li)) continue;
      const marker = ordered ? index + '. ' : '- ';
      index += 1;
      push(indent + marker + inlinePartOfItem(li));
      emitNestedLists(li, indent + '  ', depth);
    }
  }

  // Inline content of one list item: its inline children plus any
  // nested block shapes except the nested lists, which the caller
  // emits separately with indentation.
  function inlinePartOfItem(li) {
    let out = '';
    for (const node of li.childNodes || []) {
      if (!node) continue;
      if (node.nodeType === 3) {
        out += String(node.nodeValue || '').replace(/\s+/g, ' ');
        continue;
      }
      if (node.nodeType !== 1) continue;
      const tag = tagName(node);
      if (tag === 'UL' || tag === 'OL') continue;
      out += ' ' + inlineOf(node);
    }
    return collapse(out);
  }

  function emitNestedLists(li, indent, depth) {
    for (const list of arrayOf(li.children)) {
      const tag = tagName(list);
      if (tag === 'UL' || tag === 'OL') {
        emitList(list, indent, depth + 1);
      }
    }
  }

  function emitTable(el) {
    let rows = [];
    try {
      // `el.rows` and not `querySelectorAll('tr')`: the collection
      // holds this table's own rows, so a table nested inside a cell
      // contributes its rows to itself instead of interleaving them
      // into the outer table's.
      rows = arrayOf(el.rows);
    } catch {
      return;
    }
    if (rows.length === 0) return;

    // The header is the first row that carries cells: an empty leading
    // row (a spacer, a template row) must not swallow the whole table,
    // which is what treating the first row as the header
    // unconditionally did.
    let headerRow = null;
    // Markdown tables need one column count for every row, and it has
    // to fit the widest row. A colspan header spans fewer cells than the
    // data rows under it, and clipping those rows to the header's cell
    // count silently dropped their trailing cells without marking the
    // readout truncated.
    let width = 0;
    for (const row of rows) {
      const count = cellCount(row);
      if (count === 0) continue;
      if (!headerRow) headerRow = row;
      if (count > width) width = count;
    }
    if (!headerRow) return;

    const header = rowCells(headerRow);
    while (header.length < width) header.push('');
    push('| ' + header.join(' | ') + ' |');
    const separator = [];
    for (let c = 0; c < width; c += 1) separator.push('---');
    push('| ' + separator.join(' | ') + ' |');
    for (const row of rows) {
      if (row === headerRow) continue;
      const cells = rowCells(row);
      while (cells.length < width) cells.push('');
      push('| ' + cells.join(' | ') + ' |');
    }
  }

  function cellCount(row) {
    try {
      const parts = row.cells;
      return parts ? parts.length : 0;
    } catch {
      return 0;
    }
  }

  function rowCells(row) {
    const cells = [];
    try {
      for (const part of arrayOf(row.cells)) {
        cells.push(escapeCell(inlineOf(part)));
      }
    } catch {}
    return cells;
  }

  function childNodesOf(el) {
    try {
      const shadow = el.shadowRoot;
      // A host with an open shadow root contributes only its shadow
      // tree, mirroring the serializer's traversal.
      if (shadow) return shadow.childNodes || [];
      if (tagName(el) === 'SLOT' && typeof el.assignedNodes === 'function') {
        return el.assignedNodes({ flatten: true }) || [];
      }
      return el.childNodes || [];
    } catch {
      state.truncated = true;
      return [];
    }
  }

  function emitBlock(el, depth) {
    if (depth >= MAX_DEPTH || outOfBudget()) {
      state.truncated = true;
      return;
    }
    state.nodes += 1;
    if (shouldSkip(el)) return;

    const tag = tagName(el);
    if (/^H[1-6]$/.test(tag)) {
      const level = headingLevel(el);
      const text = inlineOf(el);
      if (text) push('#'.repeat(level) + ' ' + text);
      return;
    }
    if (tag === 'P') {
      pushParagraph(inlineOf(el));
      return;
    }
    if (tag === 'UL' || tag === 'OL') {
      emitList(el, '', depth);
      return;
    }
    if (tag === 'TABLE') {
      emitTable(el);
      return;
    }
    if (tag === 'PRE') {
      emitCode(el);
      return;
    }
    if (tag === 'BLOCKQUOTE') {
      prefixed('> ', () => { emitChildren(el, depth); });
      return;
    }
    if (tag === 'HR') {
      push('---');
      return;
    }
    if (tag === 'IMG') {
      const src = absoluteUrl(attribute(el, 'src') || '');
      if (src) {
        const alt = collapse(attribute(el, 'alt') || '');
        push('![' + alt.replace(/\]/g, '\\]') + '](' + src + ')');
      }
      return;
    }
    if (tag === 'A' && attribute(el, 'href')) {
      pushParagraph(inlineOf(el));
      return;
    }
    if (tag === 'DT' || tag === 'DD') {
      pushParagraph(inlineOf(el));
      return;
    }
    emitChildren(el, depth);
  }

  // Inline elements that belong to the paragraph around them instead of
  // forming a block of their own. Anything unlisted that does not
  // compute to an inline display starts a new block, like before.
  const INLINE_TAGS = new Set([
    'A', 'ABBR', 'B', 'BDI', 'BDO', 'BR', 'CITE', 'CODE',
    'DATA', 'DFN', 'EM', 'I', 'IMG', 'KBD', 'MARK', 'PICTURE',
    'Q', 'RP', 'RT', 'RUBY', 'S', 'SAMP', 'SMALL', 'SPAN',
    'STRONG', 'SUB', 'SUP', 'TIME', 'U', 'VAR', 'WBR'
  ]);

  function isInlineLevel(el) {
    if (INLINE_TAGS.has(tagName(el))) return true;
    try {
      const display = window.getComputedStyle(el).display;
      return typeof display === 'string' && display.indexOf('inline') === 0;
    } catch {
      return false;
    }
  }

  function emitChildren(el, depth) {
    const nodes = childNodesOf(el);
    // Consecutive inline children accumulate into ONE paragraph: a
    // `<div>Hello <b>world</b> again</div>` is one sentence, not three
    // (stray block text renders as plain, whitespace-collapsed
    // paragraphs). Only a block-level child breaks
    // the run.
    let pending = '';
    for (const node of nodes) {
      if (state.truncated) break;
      if (!node) continue;
      if (node.nodeType === 3) {
        pending += ' ' + String(node.nodeValue || '');
        continue;
      }
      if (node.nodeType !== 1) continue;
      if (isInlineLevel(node)) {
        // Through inlineNodes, not inlineOf: the element itself must
        // hit its own branch, so a link renders as [text](href) and
        // emphasis as **bold** even outside a `<p>`.
        pending += ' ' + inlineNodes([node], 0);
        continue;
      }
      pushParagraph(pending);
      pending = '';
      emitBlock(node, depth + 1);
    }
    pushParagraph(pending);
  }

  function joinBlocks() {
    const body = state.blocks.join('\n\n');
    const cleaned = body.replace(/\n{3,}/g, '\n\n').replace(/^\n+/, '');
    if (cleaned.length > MAX_CHARS) {
      // The '\n\n' join separators can push the total past the cap even
      // when no single push did; a clip without the flag would present
      // cut output as complete.
      state.truncated = true;
      return clip(cleaned, MAX_CHARS);
    }
    return cleaned;
  }

  let title = '';
  try {
    title = clip(collapse(document.title), TITLE_LIMIT);
  } catch {
    state.truncated = true;
  }

  const envelope = {
    version: VERSION,
    truncated: state.truncated,
    title: title,
    markdown: ''
  };

  try {
    const root = document.body || document.documentElement;
    if (root) emitBlock(root, 0);
    envelope.markdown = joinBlocks();
  } catch {
    state.truncated = true;
    envelope.markdown = joinBlocks();
  }
  envelope.truncated = state.truncated;
  return envelope;
})();
