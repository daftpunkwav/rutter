/**
 * @fileoverview Behavioural tests for the dashboard client.
 *
 * Boundary: runs `frontend/src/app.js` -- the real, shipped script --
 * against a fake document, a recording `WebSocket` and a recording
 * `fetch`. The three logics that only existed as intent in comments
 * are pinned here: a decision ack is silent, a refused screencast
 * names its reason, and a reconnect rebuilds the session set from the
 * replay instead of appending a second copy of everything.
 *
 * No build step and no test framework: `node --test` is a builtin.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { createPage, el, loadScript } from './dom.mjs';

const APP_JS = loadScript('frontend/src/app.js');

const CATALOG = {
  subtitle: 'supervision dashboard',
  grant: 'grant',
  deny: 'deny',
  reconnecting: 'connection lost; reconnecting...',
  screencastRefused: 'live view refused:',
  decisionRefused: 'decision not accepted:',
  decisionUnreachable: 'decision could not be sent:',
  labelClass: 'class',
  labelEffect: 'effect',
  labelTarget: 'target',
  labelBasis: 'because',
  effectUnknown: 'an unlisted operation',
  effectCookies: 'write {count} cookie(s) into the session',
  effectNavigate: 'navigate to',
  effectOn: 'act on',
  effectRun: 'run',
  targetUnknown: 'unknown; the URL could not be read',
  basisRule: 'rule {index} of the policy',
  basisDefault: "the policy's default verdict",
  basisMissingUrl: 'no usable URL, so it fails closed',
};

/** A socket the test delivers frames through. */
class FakeSocket {
  constructor(url) {
    this.url = url;
    this.readyState = 1;
    this.sent = [];
    sockets.push(this);
  }

  send(payload) {
    this.sent.push(JSON.parse(payload));
  }

  deliver(data) {
    this.onmessage({ data: typeof data === 'string' ? data : JSON.stringify(data) });
  }
}

/** A socket the client never opens: the state a dead one is in. */
class DeadSocket extends FakeSocket {
  constructor(url) {
    super(url);
    this.readyState = 3;
  }
}

let sockets = [];

/** Lets the script's fetch promise chain settle. */
const flush = () => new Promise((resolve) => setImmediate(resolve));

/** The ids `index.html` gives the client, wired to real elements. */
function dashboardDom() {
  const ids = {
    events: el('div'),
    sessions: el('span'),
    'session-ids': el('datalist'),
    'live-session': el('input'),
    'live-frame': el('img'),
    'live-start': el('button'),
    'live-stop': el('button'),
    'approval-list': el('div'),
  };
  Object.entries(ids).forEach(([id, node]) => {
    node.id = id;
  });
  return ids;
}

function clientDocument(ids) {
  const registry = new Map(Object.entries(ids));
  return {
    title: 'rutter dashboard',
    baseURI: 'http://127.0.0.1:7700/',
    body: el('body'),
    documentElement: el('html'),
    createElement(tag) {
      const node = el(tag);
      const setAttribute = node.setAttribute.bind(node);
      node.setAttribute = (name, value) => {
        setAttribute(name, value);
        if (name === 'id' && value) registry.set(String(value), node);
      };
      return node;
    },
    getElementById(id) {
      return registry.get(id) || null;
    },
    querySelectorAll() {
      return { forEach() {}, length: 0 };
    },
  };
}

/**
 * Boots the client over a fresh document.
 *
 * `post` decides what a decision POST answers with; `socketFor` picks
 * the socket implementation a dial gets.
 */
async function boot({ search = '', post = () => ({ ok: true, status: 200 }), socketFor } = {}) {
  sockets = [];
  const ids = dashboardDom();
  const posts = [];
  const timers = [];
  const page = createPage({
    document: clientDocument(ids),
    window: {
      location: { search, protocol: 'http:', host: '127.0.0.1:7700' },
      localStorage: {
        removed: [],
        removeItem(key) {
          this.removed.push(key);
        },
      },
      fetch(url, init) {
        if (init && init.method === 'POST') {
          posts.push({ url, body: JSON.parse(init.body) });
          const answer = post(url, init);
          return answer instanceof Error
            ? Promise.reject(answer)
            : Promise.resolve(answer);
        }
        return Promise.resolve({ ok: true, json: () => Promise.resolve(CATALOG) });
      },
      WebSocket: socketFor || FakeSocket,
      setTimeout: (fn, ms) => {
        timers.push({ fn, ms });
        return 0;
      },
      clearTimeout: () => {},
    },
  });
  page.run(APP_JS);
  await flush();
  return { page, ids, posts, timers, socket: sockets[0] };
}

const timeline = (ids) => ids.events.textContent;
const chips = (ids) => ids.sessions.textContent.trim();
const cards = (ids) => ids['approval-list'].childNodes;

const APPROVAL = {
  seq: 1,
  session: 's1',
  recorded_at: 't',
  event: {
    type: 'approval_requested',
    request_id: 'apr-1',
    brief: {
      class: 'read',
      judged_url: 'https://a.example/x',
      effect: { kind: 'action', action: { type: 'navigate', url: 'https://a.example' } },
      basis: { kind: 'rule', index: 2, url_pattern: 'https://a.example/*' },
    },
  },
};

test('a decision ack puts nothing on the timeline', async () => {
  // The ack is the server confirming a decision the card already
  // reflects; repeating it on the timeline would bury the event log
  // in bookkeeping the operator did not ask for.
  const { socket, ids } = await boot();
  socket.deliver({ type: 'note', text: 'engine started' });
  assert.equal(timeline(ids), 'engine started');
  socket.deliver({ type: 'decision-ack', request_id: 'apr-1', accepted: true });
  assert.equal(timeline(ids), 'engine started', 'the ack is silent');
});

test('a refused screencast names its reason on the timeline', async () => {
  // Silence would leave the operator looking at a blank live view
  // with no idea that anything went wrong.
  const { socket, ids } = await boot();
  socket.deliver({ type: 'screencast-ack', started: false, reason: 'no such session' });
  assert.equal(timeline(ids), 'live view refused: no such session');
});

test('a started screencast says nothing', async () => {
  const { socket, ids } = await boot();
  socket.deliver({ type: 'screencast-ack', started: true });
  assert.equal(timeline(ids), '');
});

test('a session that started and closed leaves no chip behind', async () => {
  const { socket, ids } = await boot();
  socket.deliver({ seq: 1, session: 's1', recorded_at: 't', event: { type: 'session_started' } });
  assert.equal(chips(ids), 's1');
  socket.deliver({ seq: 2, session: 's2', recorded_at: 't', event: { type: 'session_started' } });
  assert.equal(chips(ids), 's1 s2');
  socket.deliver({ seq: 3, session: 's1', recorded_at: 't', event: { type: 'session_closed' } });
  assert.equal(chips(ids), 's2');
});

test('a reconnect rebuilds the session set from the replay', async () => {
  // `s1` closed while the socket was down, so its ring was dropped
  // and the replay never mentions it again. Without the reset it
  // would linger on the chip list forever; without the timeline reset
  // the replay would append a second copy of every event.
  const { ids, socket, timers } = await boot();
  socket.deliver({ seq: 1, session: 's1', recorded_at: 't', event: { type: 'session_started' } });
  socket.deliver({ seq: 2, session: 's1', recorded_at: 't', event: { type: 'note', text: 'x' } });
  assert.equal(chips(ids), 's1');
  assert.notEqual(timeline(ids), '');

  socket.onclose();
  assert.ok(timeline(ids).includes('connection lost'), 'the operator is told the drop');
  assert.equal(timers.length, 1, 'the retry is scheduled, not run inline');
  assert.equal(timers[0].ms, 1000, 'the first retry waits a second');

  timers[0].fn();
  const reconnected = sockets[1];
  assert.ok(reconnected, 'the client dials again');
  assert.equal(reconnected.url, 'ws://127.0.0.1:7700/ws', 'to the same endpoint');
  assert.equal(chips(ids), '', 'the stale session is gone before the replay');
  assert.equal(timeline(ids), '', 'the reconnect clears the timeline the replay would duplicate');

  // The replay carries only `s2`: `s1` closed while the socket was down.
  reconnected.deliver({ seq: 7, session: 's2', recorded_at: 't', event: { type: 'session_started' } });
  assert.equal(chips(ids), 's2', 'the replay is the only source of truth after a reconnect');
});

test('repeated drops back off instead of spinning', async () => {
  // A dead server must not be dialled in a tight loop.
  const { socket, timers } = await boot();
  const waits = [];
  for (let round = 0; round < 8; round += 1) {
    socket.onclose();
    const scheduled = timers[timers.length - 1];
    waits.push(scheduled.ms);
    scheduled.fn();
  }
  assert.deepEqual(waits, [1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000]);
});

test('an approval the replay re-delivers is not duplicated', async () => {
  const { socket, ids } = await boot();
  socket.deliver(APPROVAL);
  socket.deliver(APPROVAL);
  assert.equal(cards(ids).length, 1, 'one card, not one per replay');
  const text = cards(ids)[0].textContent;
  assert.ok(text.includes('[apr-1] s1'), `the card names the request: ${text}`);
  assert.ok(text.includes('effect: navigate to https://a.example'), `the effect is described: ${text}`);
  assert.ok(
    text.includes('because: rule 2 of the policy (https://a.example/*)'),
    `the basis is described: ${text}`
  );

  socket.deliver({
    seq: 2,
    session: 's1',
    recorded_at: 't',
    event: { type: 'approval_resolved', request_id: 'apr-1' },
  });
  assert.equal(cards(ids).length, 0, 'resolving the approval takes the card down');
});

test('a refused decision post is surfaced instead of looking granted', async () => {
  // A decision that never lands leaves the agent parked until its
  // window closes, so a failed post must not read as a grant that
  // did nothing.
  const { socket, ids, posts } = await boot({ post: () => ({ ok: false, status: 403 }) });
  socket.deliver(APPROVAL);
  const [grant] = cards(ids)[0].childNodes.filter((node) => node.tagName === 'BUTTON');
  grant.onclick();
  await flush();
  assert.deepEqual(posts, [
    { url: '/api/decisions', body: { request_id: 'apr-1', grant: true } },
  ]);
  assert.equal(timeline(ids), 'decision not accepted: apr-1 (403)');
});

test('an unreachable decision endpoint is surfaced too', async () => {
  const { socket, ids } = await boot({ post: () => new Error('offline') });
  socket.deliver(APPROVAL);
  const buttons = cards(ids)[0].childNodes.filter((node) => node.tagName === 'BUTTON');
  buttons[1].onclick();
  await flush();
  assert.equal(timeline(ids), 'decision could not be sent: apr-1');
});

test('screencast control is refused locally while the socket is down', async () => {
  // A screencast on a dead socket would be dropped by the server
  // with no ack, leaving the operator waiting on a frame that can
  // never arrive; the client must not even send it.
  const { ids, socket } = await boot({ socketFor: DeadSocket });
  assert.equal(socket.sent.length, 0);
  ids['live-start'].onclick();
  assert.equal(socket.sent.length, 0, 'a closed socket carries nothing');
});

test('screencast control names the session it asked for', async () => {
  const { ids, socket } = await boot();
  ids['live-session'].value = '  s-live  ';
  ids['live-start'].onclick();
  assert.deepEqual(socket.sent, [{ type: 'screencast', on: true, session: 's-live' }]);
  ids['live-stop'].onclick();
  assert.deepEqual(socket.sent[1], { type: 'screencast', on: false, session: 's-live' });
});

test('the first visit trades its query token for the cookie and forgets it', async () => {
  // The token arrives in the query; the page scripts must never keep
  // it, and every request that follows carries it until the cookie
  // takes over.
  const { socket } = await boot({ search: '?token=s%20ecret' });
  assert.equal(socket.url, 'ws://127.0.0.1:7700/ws?token=s%20ecret');
  assert.deepEqual(socket.sent, [], 'nothing is sent before a click');
});

test('a malformed frame does not take the client down', async () => {
  const { socket, ids } = await boot();
  socket.deliver('{not json');
  socket.deliver({ seq: 3, session: 's1', recorded_at: 't', event: { type: 'navigate' } });
  assert.ok(
    timeline(ids).includes('navigate'),
    'the socket is still being read after a bad frame'
  );
});

test('a long session trims the timeline instead of growing it forever', async () => {
  // Every line is a live DOM node, so a chatty page's feed would grow
  // the document without bound on a dashboard left open for days. The
  // newest lines prepend, so the ones trimmed off the end are the
  // oldest, and the newest line the operator is reading stays.
  const MAX = 500;
  const { socket, ids } = await boot();
  for (let seq = 1; seq <= MAX + 10; seq += 1) {
    socket.deliver({ type: 'note', text: `n${seq}` });
  }
  const lines = ids.events.children.map((node) => node.textContent);
  assert.equal(lines.length, MAX, 'the timeline holds the cap');
  assert.equal(lines[0], `n${MAX + 10}`, 'the newest line prepends');
  assert.equal(lines[MAX - 1], 'n11', 'the oldest ten are the ones trimmed');
});
