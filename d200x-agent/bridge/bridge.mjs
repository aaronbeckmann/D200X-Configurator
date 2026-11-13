// AgentDeck tile bridge for the deckd firmware.
//
// A port of AgentDeck's own Ulanzi Studio plugin (plugin-ulanzi/src/app.ts, MIT) with the
// Studio SDK replaced by a line protocol on stdin/stdout, driven by d200x-agent:
//
//   stdin  (agent -> bridge):  {"cmd":"positions","keys":[0,1,...,13]}   which deck keys are tiles
//                              {"cmd":"press","key":3}                   a tile was pressed
//                              {"cmd":"release","key":3}                 released (voice hold)
//   stdout (bridge -> agent):  {"ev":"tile","key":3,"sig":"...","png":"<base64>"}
//                              {"ev":"anim","key":3,"sig":"...","delay":145,"frames":["<base64>",...]}
//                              {"ev":"log","msg":"..."}
//
// Tiles are AgentDeck's SVG renderers (@agentdeck/shared) rasterised with resvg-wasm at
// TILE px; animated tiles follow the plugin's 24-frame schedule, phase-shifted per key.

import { createRequire } from 'node:module';
import { readFileSync, existsSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';
import { createHash } from 'node:crypto';
import {
  buildSessionDeck, SESSION_SLOT_ANIM_CYCLE, sortSessions, SessionSettingsRequestTracker,
  dotDeckPresentation, voiceCommandForAction, nextClaudeWeeklyMode, nextZaiPairMode,
} from '@agentdeck/shared';

const require = createRequire(import.meta.url);
const { initWasm, Resvg } = require('@resvg/resvg-wasm');
const HERE = dirname(fileURLToPath(import.meta.url));

const TILE = 150;
// Frames per orbit cycle: AgentDeck's plugin uses 24 (145 ms); deckd decodes frames itself, so more is smooth.
const ANIM_FRAMES = Math.max(12, Math.min(96, Number(process.env.AGENTDECK_ANIM_FRAMES) || 48));
const SD_MS_PER_ANIM_UNIT = 150;
const ANIM_STEP = +(SESSION_SLOT_ANIM_CYCLE / ANIM_FRAMES).toFixed(3);
const ANIM_DELAY_MS = Math.round((SESSION_SLOT_ANIM_CYCLE * SD_MS_PER_ANIM_UNIT) / ANIM_FRAMES);
const ANIM_PROBE_FRAME = +(SESSION_SLOT_ANIM_CYCLE / 2).toFixed(3);
const PHASE_STRIDE = 7;
const animFrameAt = (i) => +(i * ANIM_STEP).toFixed(3);
function phaseOffsetFor(key) {
  const [col, row] = key.split('_').map(Number);
  return ((row * 5 + col) * PHASE_STRIDE) % ANIM_FRAMES;
}
function frameOrderFor(key) {
  const off = phaseOffsetFor(key);
  return Array.from({ length: ANIM_FRAMES }, (_, i) => (i + off) % ANIM_FRAMES);
}

const out = (o) => process.stdout.write(JSON.stringify(o) + '\n');
const log = (msg) => out({ ev: 'log', msg });

// ---- raster ----
const FONTS = ['IBMPlexSans-Regular.ttf', 'IBMPlexSans-Bold.ttf', 'JetBrainsMono-Regular.ttf', 'JetBrainsMono-Bold.ttf']
  .map((f) => join(HERE, 'resources', 'fonts', f)).filter(existsSync).map((p) => new Uint8Array(readFileSync(p)));
await initWasm(readFileSync(require.resolve('@resvg/resvg-wasm/index_bg.wasm')));
if (FONTS.length === 0) log('no fonts in resources/fonts: tiles will have no text');
const pngCache = new Map();
function svgToPng(svg) {
  let b = pngCache.get(svg);
  if (b) return b;
  const r = new Resvg(svg, { fitTo: { mode: 'height', value: TILE }, font: { fontBuffers: FONTS, loadSystemFonts: false, defaultFontFamily: 'IBM Plex Sans' } });
  b = Buffer.from(r.render().asPng()).toString('base64');
  if (pngCache.size > 400) pngCache.delete(pngCache.keys().next().value);
  pngCache.set(svg, b);
  return b;
}
const sigOf = (s) => createHash('sha1').update(s).digest('hex').slice(0, 10);

// ---- deck key <-> AgentDeck position ----
// deckd keys 0..12 are the 5x3 grid (row-major), 13 is the wide key (bottom row, right).
const posOf = (k) => (k === 13 ? '3_2' : `${k % 5}_${Math.floor(k / 5)}`);
let tileKeys = [];            // deck keys that carry AgentDeck tiles
const positions = () => tileKeys.map(posOf);
const keyOfPos = (p) => tileKeys.find((k) => posOf(k) === p);

// ---- daemon ----
function discoverPort() {
  try {
    const { port } = JSON.parse(readFileSync(join(homedir(), '.agentdeck', 'daemon.json'), 'utf8'));
    if (Number.isInteger(port) && port > 0) return port;
  } catch { /* default */ }
  return 9120;
}
let ws = null;
let connected = false;
function daemonSend(cmd) {
  if (ws && connected) ws.send(JSON.stringify(cmd));
  else log(`send(${cmd.type}) dropped: daemon not connected`);
}
function connectDaemon() {
  const port = Number(process.env.AGENTDECK_PORT) || discoverPort();
  try {
    ws = new WebSocket(`ws://127.0.0.1:${port}`);
  } catch (e) {
    log(`connect failed: ${e}`);
    setTimeout(connectDaemon, 5000);
    return;
  }
  ws.onopen = () => {
    connected = true;
    log(`daemon connected on ${port}`);
    ws.send(JSON.stringify({ type: 'client_register', clientType: 'ulanzi-plugin', clientLabel: 'Ulanzi D200X (deckd)', devices: [{ id: 'd200h', name: 'Ulanzi D200X (deckd)', family: 'd200h', columns: 5, rows: 3 }] }));
    store.setConnected(true);
    ws.send(JSON.stringify({ type: 'query_usage' }));
    invalidate();
    scheduleRender();
  };
  ws.onmessage = (m) => {
    let ev;
    try { ev = JSON.parse(m.data); } catch { return; }
    onDaemonEvent(ev);
  };
  ws.onclose = () => {
    const was = connected;
    connected = false;
    ws = null;
    if (was) log('daemon disconnected');
    store.setConnected(false);
    view = { mode: 'list', page: 0 };
    scheduleRender();
    setTimeout(connectDaemon, 5000);
  };
  ws.onerror = () => { /* onclose follows */ };
}

// ---- state store (plugin-ulanzi/src/state-store.ts) ----
function dedupeById(rows) {
  const m = new Map();
  for (const r of rows) if (r && typeof r.id === 'string') m.set(r.id, r);
  return [...m.values()];
}
class StateStore {
  lastState = { state: 'IDLE' };
  sessionStates = new Map();
  sessions = [];
  dot = null;
  usage = {};
  voiceState = 'idle';
  connected = false;
  pendingReviewUntil = new Map();
  settings = new SessionSettingsRequestTracker();
  constructor() { this.settings.onChanged = () => scheduleRender(); }
  beginSettingsQuery(id) { return this.settings.query(id); }
  beginSettingMutation(id, key, value) { return this.settings.set(id, key, value); }
  cancelSettings() { this.settings.cancel(); }
  settingsFor(id) { return this.settings.snapshot(id); }
  markReviewPending(id) { this.pendingReviewUntil.set(id, Date.now() + 20000); }
  setConnected(c) { this.connected = c; if (!c) { this.dot = null; this.settings.disconnect(); } }
  prepareFocus(id) { this.settings.cancel(); this.sessionStates.delete(id); }
  eventSessionId(e) { return (typeof e.focusedSessionId === 'string' && e.focusedSessionId) || (typeof e.sessionId === 'string' && e.sessionId) || undefined; }
  isObserved(id) { return this.sessions.find((s) => s.id === id)?.controlMode === 'observed'; }
  sessionBase(id) {
    const s = this.sessions.find((x) => x.id === id);
    if (!s) return undefined;
    return { state: s.state ?? (s.alive ? 'IDLE' : 'DISCONNECTED'), sessionId: id, focusedSessionId: '', projectName: s.projectName, agentType: s.agentType, modelName: s.modelName, effortLevel: s.effortLevel, permissionMode: s.permissionMode, mode: s.permissionMode, currentTool: s.currentTool, question: s.question, promptType: s.promptType, options: s.options ?? [] };
  }
  apply(ev) {
    const e = ev;
    switch (ev.type) {
      case 'sessions_list':
        this.dot = ev.dot ?? null;
        this.sessions = dedupeById(e.sessions ?? []);
        for (const id of this.sessionStates.keys()) if (!this.sessions.some((s) => s.id === id)) this.sessionStates.delete(id);
        return true;
      case 'state_update': {
        this.lastState = e;
        const id = this.eventSessionId(e);
        if (id && !this.isObserved(id)) this.sessionStates.set(id, { ...e, sessionId: id });
        return true;
      }
      case 'prompt_options': {
        const id = this.eventSessionId(e);
        if (!id) return false;
        const base = this.sessionStates.get(id) ?? this.sessionBase(id) ?? { sessionId: id };
        this.sessionStates.set(id, { ...base, options: e.options, promptType: e.promptType, question: e.question });
        return true;
      }
      case 'usage_update': this.usage = e; return true;
      case 'voice_state': {
        const st = typeof e.state === 'string' ? e.state : 'idle';
        if (st === this.voiceState) return false;
        this.voiceState = st;
        return true;
      }
      case 'session_settings': return this.settings.accept(ev) !== undefined;
      case 'review_status': {
        const id = typeof e.sessionId === 'string' ? e.sessionId : '';
        return !!(id && e.status !== 'running' && this.pendingReviewUntil.delete(id));
      }
      default: return false;
    }
  }
  roster() { return this.sessions; }
  sessionsWithPendingReview() {
    if (this.pendingReviewUntil.size === 0) return this.sessions;
    const now = Date.now();
    return this.sessions.map((s) => {
      const until = this.pendingReviewUntil.get(s.id);
      if (until == null) return s;
      if (s.reviewStatus != null || now > until) { this.pendingReviewUntil.delete(s.id); return s; }
      return { ...s, reviewStatus: 'running' };
    });
  }
  toLayoutInput(selectedId) {
    if (!this.connected) return { state: 'DISCONNECTED', daemonConnected: false, allSessions: [] };
    let totalTokens = this.usage.totalTokens;
    if (totalTokens == null && (this.usage.inputTokens != null || this.usage.outputTokens != null)) totalTokens = (this.usage.inputTokens ?? 0) + (this.usage.outputTokens ?? 0);
    totalTokens ??= this.lastState.totalTokens ?? 0;
    const stale = this.usage.usageStale === true;
    const usageKnown = !stale && (this.usage.fiveHourPercent != null || this.usage.sevenDayPercent != null);
    const base = selectedId ? (this.sessionBase(selectedId) ?? {}) : {};
    const snap = selectedId ? (this.sessionStates.get(selectedId) ?? {}) : {};
    const selected = selectedId ? (this.isObserved(selectedId) ? { ...snap, ...base } : { ...base, ...snap }) : this.lastState;
    return {
      ...selected,
      daemonConnected: this.connected,
      dot: this.dot,
      allSessions: this.sessionsWithPendingReview(),
      totalTokens,
      totalCost: this.usage.totalCost ?? selected.totalCost ?? 0,
      fiveHourPercent: this.usage.fiveHourPercent ?? (this.usage.claudeUsageSnapshot === true ? undefined : selected.fiveHourPercent),
      sevenDayPercent: this.usage.sevenDayPercent ?? (this.usage.claudeUsageSnapshot === true ? undefined : selected.sevenDayPercent),
      fiveHourResetsAt: this.usage.fiveHourResetsAt,
      sevenDayResetsAt: this.usage.sevenDayResetsAt,
      codexRateLimits: this.usage.codexRateLimits,
      zaiRateLimits: this.usage.zaiRateLimits,
      scopedLimits: this.usage.scopedLimits,
      usageKnown,
    };
  }
}
const store = new StateStore();

// sticky list order (plugin-ulanzi/src/session-order.ts)
class StickyOrder {
  firstSeen = new Map(); seq = 0;
  update(sessions) {
    const present = new Set();
    for (const s of sessions) { present.add(s.id); for (const id of s.foldedSessionIds ?? []) present.add(id); }
    for (const s of sortSessions(sessions.filter((s) => !this.firstSeen.has(s.id)))) this.firstSeen.set(s.id, this.seq++);
    for (const s of sessions) for (const id of s.foldedSessionIds ?? []) if (!this.firstSeen.has(id)) this.firstSeen.set(id, this.firstSeen.get(s.id));
    if (this.firstSeen.size > 256) for (const [id] of [...this.firstSeen.entries()].filter(([id]) => !present.has(id)).sort((a, b) => a[1] - b[1])) { if (this.firstSeen.size <= 256) break; this.firstSeen.delete(id); }
    return [...present].filter((id) => this.firstSeen.has(id)).sort((a, b) => this.firstSeen.get(a) - this.firstSeen.get(b));
  }
}
const sticky = new StickyOrder();
let sessionOrder;
let claudeWeeklyMode = 'both';
let zaiPairMode = 'both';
let view = { mode: 'list', page: 0 };

// ---- signature (plugin-ulanzi/src/deck-signature.ts) ----
function deckSignature(ev) {
  const sessions = (ev.allSessions ?? []).map((s) => `${s.id}:${s.state ?? ''}:${s.currentTool ?? ''}:${s.modelName ?? ''}:${JSON.stringify([s.effortLevel, s.permissionMode, s.activity, s.goal, s.contextPercent, s.subagents, s.backgroundTaskCount, s.backgroundStandbyCount])}:${s.reviewStatus ?? ''}:${s.reviewRisk ?? ''}:${s.reviewFindings ?? ''}:${s.liveAnswerable ?? ''}:${s.question ?? ''}:${s.promptType ?? ''}:${s.askGroupIndex ?? ''}:${(s.options ?? []).map((o) => o.label ?? '').join('/')}`).join(',');
  const opts = (ev.options ?? []).map((o) => o.label ?? '').join('/');
  const usage = JSON.stringify([ev.fiveHourPercent, ev.sevenDayPercent, ev.usageKnown, ev.fiveHourResetsAt, ev.sevenDayResetsAt, ev.scopedLimits, ev.codexRateLimits, ev.zaiRateLimits]);
  return [ev.state, ev.mode, ev.focusedSessionId ?? ev.sessionId ?? '', ev.requestId ?? '', ev.promptType ?? '', ev.currentTool ?? '', ev.toolInput ?? '', ev.modelName ?? '', ev.question ?? '', ev.navigable ?? '', ev.cursorIndex ?? '', opts, usage, sessions, JSON.stringify(ev.dot ?? null), ev.dot ? dotDeckPresentation(ev.dot).label : ''].join('|');
}

// ---- layout ----
function layoutInput() {
  return store.toLayoutInput(view.mode === 'detail' ? view.openSessionId : undefined);
}
const openSettings = () => (view.mode === 'detail' && view.openSessionId ? store.settingsFor(view.openSessionId) : undefined);
function deckFor(animFrame, animated) {
  return buildSessionDeck(layoutInput(), { ...view, claudeWeeklyMode, zaiPairMode, animFrame, animated, showUsage: true, voiceState: store.voiceState, settings: openSettings(), sessionOrder }, positions());
}
let deckVersion = 0;
let frameMemo = null;
function frameDecks() {
  if (frameMemo?.version !== deckVersion) frameMemo = { version: deckVersion, decks: Array.from({ length: ANIM_FRAMES }, (_, i) => deckFor(animFrameAt(i), true)) };
  return frameMemo.decks;
}

const lastSig = new Map();      // deck key -> tile signature last emitted
let lastDeckSig = '';
let lastOpenQuestion = '';
function invalidate() { lastSig.clear(); lastDeckSig = ''; }

let renderTimer = null;
let lastRenderAt = 0;
function scheduleRender() {
  if (renderTimer) return;
  const wait = Math.max(0, 120 - (Date.now() - lastRenderAt));
  renderTimer = setTimeout(() => { renderTimer = null; lastRenderAt = Date.now(); renderAll(); }, wait);
}

const animPending = new Set();  // deck keys awaiting frames
let animDraining = false;
const animCache = new Map();    // frame-0 svg -> frames (base64 array)
async function drainAnim() {
  if (animDraining) return;
  animDraining = true;
  try {
    while (animPending.size > 0) {
      const key = animPending.values().next().value;
      animPending.delete(key);
      const pos = posOf(key);
      const order = frameOrderFor(pos);
      const svgs = order.map((f) => frameDecks()[f].get(pos)?.svg);
      if (svgs.some((s) => !s)) continue;
      const cacheKey = svgs[0];
      let frames = animCache.get(cacheKey);
      if (!frames) {
        frames = [];
        let abandoned = false;
        for (const s of svgs) {
          frames.push(svgToPng(s));
          await new Promise((r) => setImmediate(r));
          if (frameDecks()[order[0]].get(pos)?.svg !== cacheKey) { abandoned = true; break; }
        }
        if (abandoned) { animPending.add(key); continue; }
        if (animCache.size > 48) animCache.delete(animCache.keys().next().value);
        animCache.set(cacheKey, frames);
      }
      out({ ev: 'anim', key, sig: sigOf(cacheKey), delay: ANIM_DELAY_MS, frames });
    }
  } catch (e) {
    log(`anim failed: ${e}`);
  } finally {
    animDraining = false;
  }
}

function renderAll() {
  if (tileKeys.length === 0) return;
  const ev = layoutInput();
  if (view.mode === 'detail' && view.openSessionId && !(ev.allSessions ?? []).some((s) => s.id === view.openSessionId)) view = { mode: 'list', page: 0 };
  const openQuestion = view.mode === 'detail' ? (ev.question ?? '') : '';
  if (openQuestion !== lastOpenQuestion) {
    lastOpenQuestion = openQuestion;
    if (view.mode === 'detail' && (view.page ?? 0) !== 0) view = { ...view, page: 0 };
  }
  const sig = JSON.stringify([view.mode, view.openSessionId, view.page ?? 0, view.picker ?? '', view.settings ?? null, store.voiceState, claudeWeeklyMode, zaiPairMode, positions().sort(), sessionOrder ?? null]) + deckSignature(ev);
  if (sig === lastDeckSig) return;
  lastDeckSig = sig;
  deckVersion++;
  const staticDeck = deckFor(0, false);
  const animBase = deckFor(0, true);
  const probe = deckFor(ANIM_PROBE_FRAME, true);
  for (const key of tileKeys) {
    const pos = posOf(key);
    const cell = staticDeck.get(pos);
    const staticSvg = cell?.svg ?? '';
    const base = animBase.get(pos)?.svg ?? '';
    const animates = !!cell && probe.get(pos)?.svg !== base;
    if (!animates) animPending.delete(key);
    const s = `${animates ? 'A' : 'S'}|${staticSvg}`;
    if (lastSig.get(key) === s) continue;
    lastSig.set(key, s);
    if (!cell) continue;
    try {
      const cached = animates ? animCache.get(frameDecks()[frameOrderFor(pos)[0]].get(pos)?.svg) : undefined;
      out({ ev: 'tile', key, sig: sigOf(staticSvg), png: svgToPng(staticSvg) });
      if (animates) {
        if (cached) out({ ev: 'anim', key, sig: sigOf(frameDecks()[frameOrderFor(pos)[0]].get(pos).svg), delay: ANIM_DELAY_MS, frames: cached });
        else animPending.add(key);
      }
    } catch (e) {
      log(`render ${pos} failed: ${e}`);
    }
  }
  if (animPending.size > 0) void drainAnim();
}

// ---- presses (app.ts onPress) ----
let voiceHeld = null; // { sessionId }
function onPress(key) {
  const pos = posOf(key);
  const cell = deckFor(0, false).get(pos);
  const action = cell?.action;
  if (!action) { log(`press ${pos} (inert)`); return; }
  switch (action.kind) {
    case 'open':
      store.prepareFocus(action.sessionId);
      view = { mode: 'detail', openSessionId: action.sessionId, page: 0 };
      daemonSend({ type: 'focus_session', sessionId: action.sessionId });
      renderAll();
      break;
    case 'back':
      store.cancelSettings();
      view = { mode: 'list', page: 0 };
      renderAll();
      break;
    case 'weekly-mode':
      claudeWeeklyMode = nextClaudeWeeklyMode(claudeWeeklyMode);
      renderAll();
      break;
    case 'zai-mode':
      zaiPairMode = nextZaiPairMode(zaiPairMode);
      renderAll();
      break;
    case 'page':
      view = { ...view, page: (view.page ?? 0) + action.delta };
      renderAll();
      break;
    case 'picker-open':
      view = { ...view, picker: action.key, page: 0 };
      if (view.openSessionId) daemonSend({ ...store.beginSettingsQuery(view.openSessionId) });
      renderAll();
      break;
    case 'picker-close':
      store.cancelSettings();
      view = { ...view, picker: undefined, page: 0 };
      renderAll();
      break;
    case 'setting-select': {
      const cmd = store.beginSettingMutation(action.sessionId, action.key, action.value);
      if (cmd) daemonSend({ ...cmd });
      renderAll();
      break;
    }
    case 'command': {
      const cmd = action.command;
      if (cmd.type === 'set_session_setting') break;
      if (cmd.type === 'voice') {
        // hold-to-talk on the plugin; here a press starts and the release stops
        const sid = cmd.sessionId ?? '';
        const start = voiceCommandForAction('voice-ptt-begin', sid || undefined);
        if (start) daemonSend({ ...start });
        voiceHeld = { sessionId: sid };
        break;
      }
      log(`press ${pos} -> ${cmd.type}`);
      daemonSend(cmd);
      if (cmd.type === 'review_run' && cmd.sessionId) { store.markReviewPending(cmd.sessionId); renderAll(); }
      break;
    }
    case 'launch':
      log('daemon offline: start it with "agentdeck daemon install"');
      break;
    default:
      break;
  }
}
function onRelease() {
  if (!voiceHeld) return;
  const stop = voiceCommandForAction('voice-ptt-end', voiceHeld.sessionId || undefined);
  voiceHeld = null;
  if (stop) daemonSend({ ...stop });
}

// ---- daemon events (app.ts) ----
let voiceErrorTimer = null;
function onDaemonEvent(ev) {
  if (ev.type === 'voice_state') {
    if (voiceErrorTimer) { clearTimeout(voiceErrorTimer); voiceErrorTimer = null; }
    if (ev.state === 'error') voiceErrorTimer = setTimeout(() => { voiceErrorTimer = null; store.voiceState = 'idle'; scheduleRender(); }, 3000);
  }
  if (ev.type === 'display_state') return;
  const pending = ev.type === 'session_settings' && view.openSessionId ? store.settingsFor(view.openSessionId)?.pending : undefined;
  if (store.apply(ev)) {
    if (ev.type === 'sessions_list') sessionOrder = sticky.update(store.roster());
    if (ev.type === 'session_settings' && pending && ev.sessionId === view.openSessionId) view = { ...view, page: 0, ...(pending === 'set' && !ev.error ? { picker: undefined } : {}) };
    scheduleRender();
  }
}

// ---- agent commands ----
createInterface({ input: process.stdin }).on('line', (line) => {
  let c;
  try { c = JSON.parse(line); } catch { return; }
  switch (c.cmd) {
    case 'positions':
      tileKeys = (c.keys ?? []).filter((k) => Number.isInteger(k) && k >= 0 && k <= 13);
      invalidate();
      scheduleRender();
      break;
    case 'press': onPress(c.key); break;
    case 'release': onRelease(); break;
    case 'refresh': invalidate(); scheduleRender(); break;
    default: break;
  }
});
process.stdin.on('end', () => process.exit(0));
connectDaemon();
log('bridge started');
