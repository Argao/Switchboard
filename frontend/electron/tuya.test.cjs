// Run with `pnpm --filter frontend test`.
const test = require('node:test')
const assert = require('node:assert/strict')
const tuya = require('./tuya.cjs')

// These values were produced by Tuya's own tuya-device-sharing-sdk from the same inputs.
const RID = '3f2b8c1e-0000-4000-8000-123456789abc'
const SESSION = { access_token: 'access-token-abc', refresh_token: 'refresh-token-xyz', endpoint: 'https://account.example' }
const HASH = 'fd770610b24099dac0608786c308ad90'
const SECRET = '3bfeac99ff438dba'
const QUERY = 'QUJDREVGR0hKS01O9vHM7VP7y2t7HqKDjTVG36GUmutu95YiKwTp3FALSVoneg=='
const SIGN = '86d9f38e9d61acdbbb40099e97c2bcd699e6e8f00100dc223a6068ed3602735c'
const RESULT =
  'QUJDREVGR0hKS01O1qiG61q8uC97QOXEjiReyqGHDfAMYnpJLHIqVVur+vdk1/2BD5/oKULSm0vKvrEwE2drACeOJC0zORdEN0yUQ3RQSpJiLux9aR0sMq7SCshYKcvgrZL33FTHTzz2+v6SZTJf0/rk891mmHTlTSdb4QDJiA=='

test('derives the request keys as the reference does', () => {
  assert.equal(tuya.hashKey(RID, SESSION.refresh_token), HASH)
  assert.equal(tuya.secretFor(RID, HASH), SECRET)
})

test('seals and signs a query as the reference does', () => {
  assert.equal(tuya.seal('{"homeId":"12345"}', SECRET, 'ABCDEFGHJKMN'), QUERY)
  const headers = { 'X-appKey': tuya.CLIENT_ID, 'X-requestId': RID, 'X-sid': '', 'X-time': '1794840000000', 'X-token': SESSION.access_token }
  assert.equal(tuya.sign(HASH, QUERY, '', headers), SIGN)
})

test('opens what the reference sealed, and nothing that was touched', () => {
  const list = JSON.parse(tuya.open(RESULT, SECRET))
  assert.deepEqual(list, [{ id: 'dev1', name: 'Plug', local_key: '0123456789abcdef', category: 'cz', online: true }])
  const touched = Buffer.from(RESULT, 'base64')
  touched[20] ^= 1
  assert.throws(() => tuya.open(touched.toString('base64'), SECRET))
  assert.throws(() => tuya.open(RESULT, 'another secret 16'.slice(0, 16)))
})

test('a random nonce still round-trips', () => {
  const sealed = tuya.seal('{"a":1}', SECRET)
  // Outgoing text is two base64 strings back to back; the answer format is one.
  const nonce = Buffer.from(sealed.slice(0, 16), 'base64')
  const rest = Buffer.from(sealed.slice(16), 'base64')
  assert.equal(tuya.open(Buffer.concat([nonce, rest]).toString('base64'), SECRET), '{"a":1}')
  assert.notEqual(tuya.seal('{"a":1}', SECRET), sealed)
})

/** A stand-in for the Tuya cloud: checks what is asked and answers sealed, as it does. */
function cloud(routes) {
  const calls = []
  const fetchImpl = async (url, init = {}) => {
    const u = new URL(url)
    calls.push({ path: u.pathname, method: init.method || 'GET', headers: init.headers, query: Object.fromEntries(u.searchParams) })
    const route = routes[u.pathname]
    if (!route) return { ok: false, status: 404, json: async () => ({}) }
    const body = typeof route === 'function' ? route(u, init) : route
    return { ok: true, status: 200, json: async () => body }
  }
  return { fetchImpl, calls }
}

/** Seals an answer the way the cloud does, for the request that asked. */
function sealedFor(init, value) {
  const secret = tuya.secretFor(init.headers['X-requestId'], tuya.hashKey(init.headers['X-requestId'], SESSION.refresh_token))
  const sealed = tuya.seal(JSON.stringify(value), secret)
  const one = Buffer.concat([Buffer.from(sealed.slice(0, 16), 'base64'), Buffer.from(sealed.slice(16), 'base64')]).toString('base64')
  return { success: true, result: one }
}

test('signs in by QR code', async () => {
  const { fetchImpl, calls } = cloud({
    '/v1.0/m/life/home-assistant/qrcode/tokens': { success: true, result: { qrcode: 'tok123' } },
    '/v1.0/m/life/home-assistant/qrcode/tokens/tok123': { success: true, t: 1, result: { ...SESSION, uid: 'u', terminal_id: 't', expire_time: 7200 } },
    '/v1.0/m/life/home-assistant/qrcode/tokens/waiting': { success: false, code: 'E', msg: 'not yet' },
  })
  const token = await tuya.qrToken(' AbC123 ', fetchImpl)
  assert.equal(token, 'tok123')
  assert.equal(tuya.qrText(token), 'tuyaSmart--qrLogin?token=tok123')
  assert.deepEqual(calls[0], { path: '/v1.0/m/life/home-assistant/qrcode/tokens', method: 'POST', headers: undefined, query: { clientid: tuya.CLIENT_ID, usercode: 'AbC123', schema: 'haauthorize' } })
  // Not confirmed in the app yet is not an error; it is asked again.
  assert.equal(await tuya.loginResult('waiting', 'AbC123', fetchImpl), null)
  // Only what later requests need is kept from the answer.
  assert.deepEqual(await tuya.loginResult('tok123', 'AbC123', fetchImpl), SESSION)
})

test('the token is never sent to an address that is not https', async () => {
  for (const endpoint of ['http://account.example', 'https://account.example/elsewhere', 'file:///etc', '', undefined]) {
    const { fetchImpl } = cloud({ '/v1.0/m/life/home-assistant/qrcode/tokens/tok': { success: true, t: 1, result: { ...SESSION, endpoint } } })
    await assert.rejects(tuya.loginResult('tok', 'AbC123', fetchImpl), /cannot be trusted/, String(endpoint))
  }
})

test('a wrong or missing user code is said plainly', async () => {
  const { fetchImpl } = cloud({ '/v1.0/m/life/home-assistant/qrcode/tokens': { success: false, code: 'USERCODE_INCORRECT', msg: 'User Code Incorrect' } })
  await assert.rejects(tuya.qrToken('nope', fetchImpl), /does not know that user code/)
  await assert.rejects(tuya.qrToken('  ', fetchImpl), /Enter the user code/)
})

test('lists the devices of every home, sockets first, without those that have no key', async () => {
  const { fetchImpl, calls } = cloud({
    '/v1.0/m/life/users/homes': (u, init) => sealedFor(init, [{ ownerId: 111, name: 'Home' }, { ownerId: 222, name: 'Office' }]),
    '/v1.0/m/life/ha/home/devices': (u, init) => {
      const secret = tuya.secretFor(init.headers['X-requestId'], tuya.hashKey(init.headers['X-requestId'], SESSION.refresh_token))
      const enc = u.searchParams.get('encdata')
      const asked = JSON.parse(tuya.open(Buffer.concat([Buffer.from(enc.slice(0, 16), 'base64'), Buffer.from(enc.slice(16), 'base64')]).toString('base64'), secret))
      // The request is signed over exactly what was sent.
      assert.equal(init.headers['X-sign'], tuya.sign(tuya.hashKey(init.headers['X-requestId'], SESSION.refresh_token), enc, '', init.headers))
      return sealedFor(
        init,
        asked.homeId === '111'
          ? [
              { id: 'lamp', name: 'Lamp', local_key: 'k-lamp', category: 'dj', online: true },
              { id: 'plug', name: 'PC plug', local_key: 'k-plug', category: 'cz', online: true },
              { id: 'hubless', name: 'Sensor', local_key: '', category: 'wsdcg', online: true },
            ]
          : [{ id: 'plug2', name: 'Another plug', local_key: 'k-plug2', category: 'cz', online: false }],
      )
    },
  })
  const list = await tuya.devices(SESSION, fetchImpl)
  assert.deepEqual(list, [
    { id: 'plug2', name: 'Another plug', key: 'k-plug2', socket: true, online: false },
    { id: 'plug', name: 'PC plug', key: 'k-plug', socket: true, online: true },
    { id: 'lamp', name: 'Lamp', key: 'k-lamp', socket: false, online: true },
  ])
  assert.equal(calls.length, 3)
  assert.equal(calls[0].headers['X-token'], SESSION.access_token)
})

test('a refusal from the cloud is an error, not an empty list', async () => {
  const { fetchImpl } = cloud({ '/v1.0/m/life/users/homes': { success: false, code: '1010', msg: 'token invalid' } })
  await assert.rejects(tuya.devices(SESSION, fetchImpl), /token invalid/)
  await assert.rejects(tuya.devices(SESSION, cloud({}).fetchImpl), /Tuya answered 404/)
})
