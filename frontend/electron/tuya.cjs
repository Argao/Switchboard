// Reads a smart plug's local key from the person's Smart Life account.
//
// A Tuya plug only answers on the local network to whoever holds its local
// key, and that key lives in the Tuya cloud. The documented way to read it
// needs a Tuya developer account. This is the other way: the person signs in
// by scanning a QR code with the Smart Life (or Tuya Smart) app, the same
// way Home Assistant's Tuya integration does, and the account's devices come
// back with their keys.
//
// It goes through the app registration Tuya gave Home Assistant, which Tuya
// has not offered to anyone else and could stop honouring. So this is a
// shortcut, and typing the address and key by hand always works without it.
//
// This is the only part of Switchboard that talks to a cloud, it runs only
// when the person asks for it, and nothing it learns is kept: the key goes
// straight to the daemon, which stores it in the OS keychain.
const crypto = require('node:crypto')

const CLIENT_ID = 'HA_3y9q4ak7g4ephrvke'
const SCHEMA = 'haauthorize'
const LOGIN = 'https://apigw.iotbing.com'
const NONCE_CHARS = 'ABCDEFGHJKMNPQRSTWXYZabcdefhijkmnprstwxyz2345678'

const hashKey = (requestId, refreshToken) => crypto.createHash('md5').update(requestId + refreshToken).digest('hex')

/** The key one request and its answer are sealed with. */
const secretFor = (requestId, hash) => crypto.createHmac('sha256', requestId).update(hash).digest('hex').slice(0, 16)

function randomNonce() {
  let n = ''
  for (let i = 0; i < 12; i++) n += NONCE_CHARS[crypto.randomInt(NONCE_CHARS.length)]
  return n
}

/** What goes out: the nonce and the sealed text, each in base64, one after the other. */
function seal(text, secret, nonce = randomNonce()) {
  const c = crypto.createCipheriv('aes-128-gcm', Buffer.from(secret), Buffer.from(nonce))
  const sealed = Buffer.concat([c.update(text, 'utf8'), c.final(), c.getAuthTag()])
  return Buffer.from(nonce).toString('base64') + sealed.toString('base64')
}

/** What comes back: nonce, sealed text and tag in one base64 string. */
function open(b64, secret) {
  const raw = Buffer.from(b64, 'base64')
  const d = crypto.createDecipheriv('aes-128-gcm', Buffer.from(secret), raw.subarray(0, 12))
  d.setAuthTag(raw.subarray(raw.length - 16))
  return Buffer.concat([d.update(raw.subarray(12, raw.length - 16)), d.final()]).toString('utf8')
}

function sign(hash, query, body, headers) {
  const head = ['X-appKey', 'X-requestId', 'X-sid', 'X-time', 'X-token']
    .filter((h) => headers[h])
    .map((h) => `${h}=${headers[h]}`)
    .join('||')
  return crypto.createHmac('sha256', hash).update(head + (query || '') + (body || '')).digest('hex')
}

async function json(res) {
  if (!res.ok) throw new Error(`Tuya answered ${res.status}`)
  return res.json()
}

/** A signed, sealed GET on the account's own endpoint. */
async function get(session, path, params, fetchImpl = fetch, requestId = crypto.randomUUID(), now = Date.now()) {
  const hash = hashKey(requestId, session.refresh_token)
  const secret = secretFor(requestId, hash)
  const query = params && Object.keys(params).length ? seal(JSON.stringify(params), secret) : ''
  const headers = { 'X-appKey': CLIENT_ID, 'X-requestId': requestId, 'X-sid': '', 'X-time': String(now), 'X-token': session.access_token }
  headers['X-sign'] = sign(hash, query, '', headers)
  const url = session.endpoint + path + (query ? `?encdata=${encodeURIComponent(query)}` : '')
  const r = await json(await fetchImpl(url, { headers, signal: AbortSignal.timeout(15000) }))
  if (!r.success) throw new Error(r.msg || 'Tuya refused the request')
  if (!r.result) return null
  const text = open(r.result, secret)
  try {
    return JSON.parse(text)
  } catch {
    return text
  }
}

/** Starts a sign-in; resolves to the token the QR code carries. */
async function qrToken(userCode, fetchImpl = fetch) {
  const code = String(userCode || '').trim()
  if (!code) throw new Error('Enter the user code from the Smart Life app')
  const url = `${LOGIN}/v1.0/m/life/home-assistant/qrcode/tokens?clientid=${CLIENT_ID}&usercode=${encodeURIComponent(code)}&schema=${SCHEMA}`
  const r = await json(await fetchImpl(url, { method: 'POST', signal: AbortSignal.timeout(15000) }))
  if (!r.success) throw new Error(r.code === 'USERCODE_INCORRECT' ? 'Smart Life does not know that user code' : r.msg || 'Tuya refused the sign-in')
  return r.result.qrcode
}

/** What the Smart Life app expects to find in the QR code. */
const qrText = (token) => `tuyaSmart--qrLogin?token=${token}`

/** The session once the person confirmed in the app; null until then. */
async function loginResult(token, userCode, fetchImpl = fetch) {
  const url = `${LOGIN}/v1.0/m/life/home-assistant/qrcode/tokens/${encodeURIComponent(token)}?clientid=${CLIENT_ID}&usercode=${encodeURIComponent(String(userCode).trim())}`
  const r = await json(await fetchImpl(url, { signal: AbortSignal.timeout(15000) }))
  if (!r.success) return null
  const { access_token, refresh_token, endpoint } = r.result
  // Every later request carries the token to this address.
  if (!/^https:\/\/[^/]+$/.test(String(endpoint))) throw new Error('Tuya answered with an address that cannot be trusted')
  return { access_token, refresh_token, endpoint }
}

/** Every device of the account that has a local key, sockets first. */
async function devices(session, fetchImpl = fetch) {
  const homes = (await get(session, '/v1.0/m/life/users/homes', null, fetchImpl)) || []
  const found = new Map()
  for (const home of homes) {
    const list = (await get(session, '/v1.0/m/life/ha/home/devices', { homeId: String(home.ownerId) }, fetchImpl)) || []
    for (const d of list) {
      if (d.local_key) found.set(d.id, { id: d.id, name: d.name || d.product_name || d.id, key: d.local_key, socket: d.category === 'cz', online: !!d.online })
    }
  }
  return [...found.values()].sort((a, b) => b.socket - a.socket || a.name.localeCompare(b.name))
}

module.exports = { hashKey, secretFor, seal, open, sign, get, qrToken, qrText, loginResult, devices, CLIENT_ID }
