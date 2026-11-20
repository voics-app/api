// Disposable compose-network acceptance fixture. Never prints credentials or tokens.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';

const api = 'http://api:8080';
const messages = 'http://messages:8082';
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function request(base, method, path, token, body) {
  const response = await fetch(`${base}${path}`, {
    method,
    headers: { ...(token ? { authorization: `Bearer ${token}` } : {}), ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const raw = await response.text();
  let value = null;
  if (raw) { try { value = JSON.parse(raw); } catch { value = null; } }
  return { status: response.status, body: value };
}
async function waitFor(label, fn, timeout = 120_000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try { last = await fn(); if (last) return last; } catch { /* dependencies may still be starting */ }
    await pause(1_000);
  }
  throw new Error(`Timed out waiting for ${label}`);
}
for (const [name, url] of [['api', `${api}/healthz`], ['messages', `${messages}/healthz`], ['gateway', 'http://gateway:4000/readyz']]) {
  await waitFor(name, async () => (await fetch(url)).ok);
}

const suffix = randomBytes(5).toString('hex');
const password = `${randomBytes(20).toString('hex')}A1!`;
async function register(label) {
  const result = await request(api, 'POST', '/api/v1/auth/register', null, {
    email: `${label}${suffix}@example.test`, username: `${label}${suffix}`, password,
  });
  assert.equal(result.status, 201, 'register');
  assert.ok(result.body?.access_token && result.body?.user?.id);
  return { token: result.body.access_token, id: result.body.user.id };
}
const owner = await register('owner');
const guest = await register('guest');
const guild = await request(api, 'POST', '/api/v1/guilds', owner.token, { name: 'Insights QA' });
assert.equal(guild.status, 201, 'create guild');
const guildId = guild.body.id;
const text = await request(api, 'POST', `/api/v1/guilds/${guildId}/channels`, owner.token, { name: 'qa-text', type: 0 });
const voice = await request(api, 'POST', `/api/v1/guilds/${guildId}/channels`, owner.token, { name: 'qa-voice', type: 2 });
assert.equal(text.status, 201, 'create text channel');
assert.equal(voice.status, 201, 'create voice channel');
const invite = await request(api, 'POST', `/api/v1/channels/${text.body.id}/invites`, owner.token, {});
assert.equal(invite.status, 200, 'create invite');
assert.equal((await request(api, 'POST', `/api/v1/invites/${invite.body.code}`, guest.token, {})).status, 200, 'join guild');

const widget = `/api/v1/guilds/${guildId}/widget`;
const widgetJson = `${widget}.json`;
assert.equal((await request(api, 'GET', widgetJson)).status, 404, 'widget defaults disabled');
assert.equal((await request(api, 'PATCH', widget, guest.token, { enabled: true })).status, 403, 'widget management permission');
assert.equal((await request(api, 'PATCH', widget, owner.token, { enabled: true, channel_id: text.body.id })).status, 200, 'enable widget');
const publicWidget = await request(api, 'GET', widgetJson);
assert.equal(publicWidget.status, 200, 'public widget');
assert.equal(publicWidget.body.id, guildId);
assert.equal(publicWidget.body.channel_id, undefined, 'widget does not disclose selected channel');

const insightsPath = `/api/v1/guilds/${guildId}/insights?period=7`;
assert.equal((await request(api, 'GET', insightsPath, guest.token)).status, 403, 'insights permission');
const insights = async () => {
  const result = await request(api, 'GET', insightsPath, owner.token);
  assert.equal(result.status, 200, 'insights');
  assert.equal(result.body.days.length, 7);
  assert.equal(result.body.message_coverage, false, 'mixed rollout remains incomplete');
  assert.equal(result.body.voice_coverage, false, 'voice gaps remain incomplete');
  return result.body.days.at(-1);
};
await pause(5_000); // Allow any guild-join system messages to settle before the baseline.
const baseline = await waitFor('message marker source', async () => {
  const day = await insights();
  return typeof day.messages === 'number' && day.message_source_initialized_at_ms ? day : null;
});
assert.equal(baseline.new_members, 2, 'two committed joins');
const sent = await request(messages, 'POST', `/api/v1/channels/${text.body.id}/messages`, owner.token, { content: 'one observed guild message' });
assert.equal(sent.status, 200, 'send normal message');
await waitFor('message marker', async () => {
  const day = await insights();
  return day.messages === baseline.messages + 1 && day.message_complete === false;
});

const ws = new WebSocket('ws://gateway:4000/gateway?v=1&encoding=json');
const frames = [];
let timer;
let sequence = null;
ws.onmessage = (event) => {
  const frame = JSON.parse(typeof event.data === 'string' ? event.data : Buffer.from(event.data).toString());
  frames.push(frame);
  if (frame.s != null) sequence = frame.s;
  if (frame.op === 10) {
    timer = setInterval(() => { if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ op: 1, d: sequence })); }, Math.max(1_000, Math.floor(frame.d.heartbeat_interval / 2)));
    ws.send(JSON.stringify({ op: 2, d: { token: guest.token, device_id: null, intents: 641, properties: { os: 'qa', client: 'qa', version: '0' } } }));
  }
};
await waitFor('gateway READY', () => frames.some((frame) => frame.t === 'READY'));
await waitFor('guild subscription', () => frames.some((frame) => frame.t === 'GUILD_CREATE' && frame.d?.id === guildId));
const voiceUpdate = (channelId) => ws.send(JSON.stringify({ op: 4, d: { guild_id: guildId, channel_id: channelId, self_mute: false, self_deaf: false, self_video: false, self_stream: false } }));
await waitFor('voice join and allocation', () => {
  if (frames.some((frame) => frame.t === 'VOICE_SERVER_UPDATE' && frame.d?.channel_id === voice.body.id)) return true;
  voiceUpdate(voice.body.id);
  return false;
}, 90_000);
const joinedAt = Date.now();
const statusPath = `/api/v1/channels/${voice.body.id}/voice-status`;
assert.equal((await request(api, 'PUT', statusPath, guest.token, { status: 'testing together' })).status, 204, 'connected member sets status');
assert.equal((await request(api, 'GET', `/api/v1/channels/${voice.body.id}`, owner.token)).body.status, 'testing together');
await waitFor('channel update dispatch', () => frames.some((frame) => frame.t === 'CHANNEL_UPDATE' && frame.d?.id === voice.body.id && frame.d?.status === 'testing together'));
await pause(17_000);
voiceUpdate(null);
await waitFor('voice leave', () => frames.some((frame) => frame.t === 'VOICE_STATE_UPDATE' && frame.d?.user_id === guest.id && frame.d?.channel_id === null));
const voiceDay = await waitFor('observed voice time', async () => {
  const day = await insights();
  return typeof day.voice_minutes === 'number' && day.voice_minutes > 0 ? day : null;
});
assert.ok(voiceDay.voice_minutes <= (Date.now() - joinedAt) / 60_000 + 0.1, 'voice count does not exceed observed call window');
assert.equal((await request(api, 'PUT', statusPath, guest.token, { status: null })).status, 403, 'disconnected member has no status permission');
assert.equal((await request(api, 'DELETE', `/api/v1/users/@me/guilds/${guildId}`, guest.token)).status, 204, 'leave guild');
assert.equal((await insights()).leavers, 1, 'committed departure counted');
clearInterval(timer);
ws.close();
console.log('PASS: normal message, widget, connected voice status and observed activity; unverified coverage remains explicit');
