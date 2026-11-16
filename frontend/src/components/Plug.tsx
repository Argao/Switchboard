import { useCallback, useEffect, useState, type CSSProperties } from 'react'
import QRCode from 'react-qr-code'
import { Plug as PlugIcon, QrCode, Warning } from '@phosphor-icons/react'
import { Button, Card, Chip, Dialog, ShapeIcon, TextField, useToast } from '@/components/ui'
import { sb, useHostState, type PlugView, type SmartLifeDevice } from '@/lib/bridge'

/** How many times the sign-in is asked about, two seconds apart, before the QR code is taken as expired. */
const QR_TRIES = 75

/** The smart plug this computer is plugged into. When to shut down, and how long the plug then waits, are chosen on the phone. */
export function PlugCard({ style }: { style?: CSSProperties }) {
  const toast = useToast()
  const [plug, setPlug] = useState<PlugView | null>(null)
  const [dialog, setDialog] = useState<'import' | 'manual' | null>(null)
  const { state, refresh } = useHostState(5000)
  const total = state?.total_shutdown

  const load = useCallback(() => {
    sb.call<PlugView>('plug').then(setPlug, () => {})
  }, [])
  useEffect(load, [load])

  const done = (p: PlugView) => {
    setDialog(null)
    setPlug(p)
    load()
    refresh()
    toast('Plug connected. Your phone now offers a total shutdown.')
  }
  const forget = async () => {
    try {
      await sb.call('forget_plug')
      setPlug({ configured: false })
      refresh()
    } catch (e) {
      toast((e as Error).message)
    }
  }
  if (!plug) return null
  return (
    <>
      <Card className="rise divide-y divide-border/60" style={style}>
        <div className="flex items-center gap-4 px-5 py-4">
          <ShapeIcon icon={<PlugIcon size={20} weight="bold" />} shape="cookie6" tone="tertiary" size={40} />
          <div className="min-w-0 flex-1">
            <p className="font-medium">{plug.configured ? `Smart plug at ${plug.host ?? 'its address'}` : 'Smart plug'}</p>
            <p className="text-label text-muted">
              {!plug.configured
                ? 'Plug this computer into a Tuya smart plug (the Smart Life app) and your phone can shut it down, then cut the power at the plug.'
                : plug.reachable
                  ? `Answering${plug.watts != null ? ` · ${plug.watts.toFixed(0)} W through it right now` : ''}. Everything else on this plug loses power too.`
                  : plug.error}
            </p>
          </div>
          {plug.configured ? (
            <>
              <Chip tone={plug.reachable ? 'on' : 'warn'}>{plug.reachable ? 'Connected' : 'Not answering'}</Chip>
              <Button variant="text" onClick={forget}>Forget</Button>
            </>
          ) : (
            <>
              <Button variant="text" onClick={() => setDialog('manual')}>Enter by hand</Button>
              <Button icon={<QrCode size={18} weight="bold" />} onClick={() => setDialog('import')}>From Smart Life</Button>
            </>
          )}
        </div>
        {plug.configured && (total?.pending_update || total?.error) && (
          <div className="flex items-center gap-4 px-5 py-4">
            <ShapeIcon icon={<Warning size={20} weight="bold" />} shape="cookie9" tone="danger" size={40} />
            <p className="min-w-0 flex-1 text-label text-muted">
              {total.pending_update ? 'An update is waiting to install when this computer shuts down. That can take longer than the plug waits, so a total shutdown is refused until it is done.' : total.error}
            </p>
          </div>
        )}
      </Card>
      {dialog === 'import' && <ImportDialog onClose={() => setDialog(null)} onDone={done} />}
      {dialog === 'manual' && <ManualDialog onClose={() => setDialog(null)} onDone={done} />}
    </>
  )
}

/** Address and local key typed in, for someone who already has them. */
function ManualDialog({ onClose, onDone }: { onClose: () => void; onDone: (p: PlugView) => void }) {
  const [host, setHost] = useState('')
  const [key, setKey] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const save = async () => {
    setBusy(true)
    setError(null)
    try {
      onDone(await sb.call<PlugView>('set_plug', host.trim() ? { host: host.trim(), key } : { key }))
    } catch (e) {
      setError((e as Error).message)
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog
      open
      onClose={onClose}
      title="Smart plug"
      actions={
        <>
          <Button variant="text" onClick={onClose}>Cancel</Button>
          <Button onClick={save} loading={busy} disabled={key.trim().length !== 16}>Connect</Button>
        </>
      }
    >
      <p className="mb-4">The plug's local key is 16 characters, read from a Tuya developer account. Leave the address empty to have this network searched for the plug.</p>
      <div className="flex flex-col gap-3">
        <TextField label="Address (optional)" value={host} onChange={setHost} maxLength={64} />
        <TextField label="Local key" value={key} onChange={setKey} onEnter={save} maxLength={16} />
      </div>
      {error && <p className="mt-3 text-danger">{error}</p>}
    </Dialog>
  )
}

/** Sign in to Smart Life by QR code, pick the plug, and let the daemon find it. */
function ImportDialog({ onClose, onDone }: { onClose: () => void; onDone: (p: PlugView) => void }) {
  const [code, setCode] = useState('')
  const [qr, setQr] = useState<string | null>(null)
  const [devices, setDevices] = useState<SmartLifeDevice[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const close = () => {
    sb.plugClose()
    onClose()
  }
  const start = async () => {
    setBusy(true)
    setError(null)
    try {
      setQr(await sb.plugLogin(code))
    } catch (e) {
      setError((e as Error).message)
    } finally {
      setBusy(false)
    }
  }
  // While the QR code is up, ask whether the person has confirmed in the app.
  useEffect(() => {
    if (!qr || devices) return
    let alive = true
    const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))
    // One question at a time: listing the devices takes longer than the pause between questions.
    ;(async () => {
      for (let asked = 0; alive && asked < QR_TRIES; asked++) {
        await wait(2000)
        if (!alive) return
        try {
          const list = await sb.plugDevices()
          if (!alive) return
          if (list) return setDevices(list)
        } catch (e) {
          if (alive) setError((e as Error).message)
          return
        }
      }
      // Tuya stops honouring the code after a couple of minutes.
      if (alive) {
        setQr(null)
        setError('The code expired before it was scanned. Continue to get a new one.')
      }
    })()
    return () => {
      alive = false
    }
  }, [qr, devices])
  const use = async (d: SmartLifeDevice) => {
    setBusy(true)
    setError(null)
    try {
      onDone(await sb.plugUse(d.id))
    } catch (e) {
      setError((e as Error).message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open
      onClose={close}
      title={devices ? 'Which one is this computer plugged into?' : 'Smart Life'}
      actions={
        <>
          <Button variant="text" onClick={close}>Cancel</Button>
          {!qr && <Button onClick={start} loading={busy} disabled={!code.trim()}>Continue</Button>}
        </>
      }
    >
      {!qr && (
        <>
          <p className="mb-4">The plug has to be in the Smart Life or Tuya Smart app. There, open Me, the settings gear, Account and Security, and copy the User Code.</p>
          <TextField label="User code" value={code} onChange={setCode} onEnter={start} maxLength={32} />
          <p className="mt-3 text-label">This signs in to Tuya once to read the plug's key. Afterwards the computer talks to the plug directly, without the internet.</p>
        </>
      )}
      {qr && !devices && (
        <>
          <div className="mx-auto mb-4 w-fit rounded-2xl bg-white p-3">
            <QRCode value={qr} size={208} level="M" bgColor="#FFFFFF" fgColor="#131317" aria-label="Smart Life sign-in QR code" />
          </div>
          <p>In Smart Life, open Me, tap the scan icon at the top, scan this code and confirm. It lasts a minute or two.</p>
        </>
      )}
      {devices && (
        <div className="flex flex-col gap-2">
          {devices.length === 0 && <p>This account has no device with a local key.</p>}
          {devices.map((d) => (
            <Button key={d.id} variant={d.socket ? 'tonal' : 'outlined'} disabled={busy} onClick={() => use(d)} className="justify-start">
              {d.name}
              {!d.online && ' (offline)'}
            </Button>
          ))}
          {busy && <p className="mt-2">Looking for it on your network…</p>}
        </div>
      )}
      {error && <p className="mt-3 text-danger">{error}</p>}
    </Dialog>
  )
}
