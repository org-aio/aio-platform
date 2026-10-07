import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

function guest() {
  const events = new Map();
  const sent = [];
  const frames = [];
  const parent = { postMessage: value => sent.push(value) };
  const window = { aioLifecycle: { activated: true } };
  const location = { hash: '', replace(value) { this.hash = value === '#' ? '' : value; } };
  const items = { dataset: { urlScroll: 'items' }, scrollHeight: 2500, clientHeight: 200, scrollWidth: 200, clientWidth: 200, scrollTop: 0, scrollLeft: 0 };
  const document = { currentScript: { dataset: { token: 'grant' } }, documentElement: {},
    scrollingElement: { scrollHeight: 600, clientHeight: 600, scrollWidth: 600, clientWidth: 600, scrollTop: 0, scrollLeft: 0 },
    querySelectorAll: () => [items], querySelector: () => items };
  const context = vm.createContext({ window, parent, location, document, Date, Map, console, setTimeout, clearTimeout,
    MutationObserver: class { observe() {} }, requestAnimationFrame: callback => frames.push(callback),
    addEventListener: (name, callback) => events.set(name, callback),
  });
  vm.runInContext(readFileSync(new URL('./navigation.js', import.meta.url), 'utf8'), context);
  const receive = data => events.get('message')({ source: parent, data: { channel: 'aio-navigation', token: 'grant', ...data } });
  return { window, location, events, sent, parent, receive, document, frames, items };
}

test('guest navigation is scoped, bounded and supports replace without exposing its asset URL', async () => {
  const g = guest();
  const pending = g.window.aioNavigation.navigate('#/files?category=application', { replace: true });
  assert.equal(g.sent[0].navigation, '#/files?category=application');
  assert.equal(g.sent[0].replace, true);
  assert.equal(Object.hasOwn(g.sent[0], 'src'), false);
  g.receive({ id: g.sent[0].id, response: g.sent[0].navigation });
  assert.equal(await pending, '#/files?category=application');
  await assert.rejects(g.window.aioNavigation.navigate('https://outside.test'), /片段/);
  await assert.rejects(g.window.aioNavigation.navigate('#' + 'x'.repeat(2048)), /片段/);
});

test('host restore updates the existing guest and does not echo a new history node', () => {
  const g = guest();
  const values = [];
  const stop = g.window.aioNavigation.onNavigationChange(value => values.push(value));
  const message = { channel: 'aio-navigation', token: 'grant', navigation: '#/details?tab=history' };
  g.events.get('message')({ source: {}, data: message });
  g.events.get('message')({ source: g.parent, data: { ...message, token: 'other' } });
  assert.equal(g.location.hash, '');
  g.receive({ navigation: message.navigation, scroll: '' });
  assert.equal(g.location.hash, message.navigation);
  g.events.get('hashchange')();
  assert.equal(g.sent.length, 0);
  assert.deepEqual(values, ['', message.navigation]);
  stop();
  g.receive({ navigation: '#/elsewhere', scroll: '' });
  assert.equal(values.length, 2);
});

test('native hash navigation is reported and inactive guest scroll is ignored', async () => {
  const g = guest();
  g.location.hash = '#/items?status=open';
  g.events.get('hashchange')();
  assert.equal(g.sent[0].navigation, g.location.hash);
  g.receive({ id: g.sent[0].id, response: g.location.hash });
  g.events.get('aio:visibility')({ detail: false });
  g.events.get('scroll')({ target: g.document });
  assert.equal(g.sent.length, 1);
});

test('empty scroll resets retained containers and an old completion cannot cancel a newer restore', () => {
  const g = guest();
  g.items.scrollTop = 420;
  g.receive({ navigation: '#/items', scroll: '' });
  g.frames.shift()();
  assert.equal(g.items.scrollTop, 0);
  g.receive({ navigation: '#/details', scroll: 'guest-items:0:300' });
  g.frames.shift()();
  g.frames.shift()();
  assert.equal(g.items.scrollTop, 300);
  assert.equal(g.sent.length, 0);
});

test('shared host bridge enforces frame, opaque origin, ticket, visibility and disposal', () => {
  const callbacks = new Map();
  const updates = [];
  const positions = [];
  const replies = [];
  let visible = true;
  const child = { postMessage: value => replies.push(value) };
  const window = { __adminUrlState: { parameter: () => '#/restored', scroll: () => 'guest-window:0:200',
    update: value => updates.push(value), recordScroll: value => positions.push(value) },
    addEventListener: (name, callback) => callbacks.set(name, callback), removeEventListener: name => callbacks.delete(name),
  };
  const frame = { contentWindow: child, closest: () => null, addEventListener() {}, removeEventListener() {} };
  const source = readFileSync(new URL('../../lib/plugin/host/src/runtime/frontend_navigation.js', import.meta.url), 'utf8');
  const mount = vm.runInNewContext(`${source}\nmountFrontendNavigation`, { window, MutationObserver: class { observe() {} disconnect() {} } });
  const mounted = mount(frame, { token: 'grant', page_id: 'page-a' }, () => true, () => visible);
  assert.equal(mounted.fragment, '#/restored');
  const event = { source: child, origin: 'null', data: { channel: 'aio-navigation', token: 'grant', id: '1', navigation: '#/new' } };
  const receive = callbacks.get('message');
  receive({ ...event, source: {} });
  receive({ ...event, origin: 'https://outside.test' });
  receive({ ...event, data: { ...event.data, token: 'other' } });
  assert.equal(updates.length, 0);
  receive(event);
  assert.equal(updates[0].page, 'page-a');
  assert.equal(updates[0].values.route, '#/new');
  receive({ ...event, data: { ...event.data, navigation: undefined, scroll: { key: 'window', x: 0, y: 200 } } });
  assert.equal(positions[0].key, 'guest-window');
  visible = false;
  receive(event);
  assert.equal(updates.length, 1);
  assert.match(replies.at(-1).error, /未激活/);
  mounted.dispose();
  receive(event);
  assert.equal(updates.length, 1);
  assert.equal(callbacks.size, 0);
});
