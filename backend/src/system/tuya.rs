//! A Tuya smart plug, spoken to directly over the local network.
//!
//! Most Wi-Fi plugs sold under local brands are Tuya devices. Each listens
//! on TCP 6668 and answers only to whoever holds its `local key`, a secret
//! the Tuya cloud hands the plug when it is paired. Nothing here goes
//! through that cloud: the key is asked for once, elsewhere, and after that
//! the computer and the plug talk on their own.
//!
//! This speaks version 3.5 of the protocol, the one current firmware uses:
//! every frame is sealed with AES-GCM, and each connection first agrees on a
//! key of its own from a random number of each side. Older plugs (3.3, 3.4)
//! frame and seal differently and are not handled yet.
//!
//! Only what a total shutdown needs is here: reading the plug, and setting
//! the timer it switches itself off by.
//!
//! A connection is opened for one request and closed again. Plugs take only
//! a few connections at a time, and their own app wants one.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload};
use aes_gcm::{Aes128Gcm, Nonce};
use anyhow::{Context, Result, anyhow, bail, ensure};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub const PORT: u16 = 6668;

const PREFIX: [u8; 4] = [0x00, 0x00, 0x66, 0x99];
const SUFFIX: [u8; 4] = [0x00, 0x00, 0x99, 0x66];
/// Prefix, two unused bytes, sequence number, command, length.
const HEADER: usize = 18;
const IV: usize = 12;
const TAG: usize = 16;
/// No frame a plug sends comes near this; a larger length is a lost stream.
const MAX_BODY: usize = 64 * 1024;

const SESSION_START: u32 = 0x03;
const SESSION_REPLY: u32 = 0x04;
const SESSION_FINISH: u32 = 0x05;
const CONTROL: u32 = 0x0d;
const QUERY: u32 = 0x10;

/// Goes in front of what a control frame carries, and of what a plug reports.
const VERSION: &[u8; 15] = b"3.5\0\0\0\0\0\0\0\0\0\0\0\0";

// The datapoints of a Tuya socket (category `cz`).
const DP_SWITCH: &str = "1";
const DP_COUNTDOWN: &str = "9";
/// Power drawn, in tenths of a watt; only on plugs that meter.
const DP_POWER: &str = "19";

const CONNECT: Duration = Duration::from_secs(3);
const ANSWER: Duration = Duration::from_secs(5);
/// How long a plug gets to acknowledge a change before it is simply asked what it is doing.
const ACK: Duration = Duration::from_secs(2);

pub type Key = [u8; 16];

fn cipher(key: &Key) -> Aes128Gcm {
    Aes128Gcm::new(key.into())
}

fn mac(key: &Key, data: &[u8]) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes a key of any length");
    m.update(data);
    m
}

/// One frame: the header in the clear but vouched for, the body sealed.
fn seal(key: &Key, seq: u32, cmd: u32, body: &[u8], iv: [u8; IV]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(HEADER + IV + body.len() + TAG + SUFFIX.len());
    out.extend_from_slice(&PREFIX);
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&cmd.to_be_bytes());
    out.extend_from_slice(&((IV + body.len() + TAG) as u32).to_be_bytes());
    let sealed = cipher(key).encrypt(Nonce::from_slice(&iv), Payload { msg: body, aad: &out[4..HEADER] }).map_err(|_| anyhow!("could not seal a frame"))?;
    out.extend_from_slice(&iv);
    out.extend_from_slice(&sealed);
    out.extend_from_slice(&SUFFIX);
    Ok(out)
}

/// How many more bytes follow a header, after checking it is one.
fn rest_len(header: &[u8; HEADER]) -> Result<usize> {
    ensure!(header[..4] == PREFIX, "that is not a Tuya 3.5 device");
    let len = u32::from_be_bytes(header[14..18].try_into().unwrap()) as usize;
    ensure!((IV + TAG..=MAX_BODY).contains(&len), "the plug sent a frame of an impossible size");
    Ok(len + SUFFIX.len())
}

/// The command and the body of a frame, given its header and what followed it.
fn open(key: &Key, header: &[u8; HEADER], rest: &[u8]) -> Result<(u32, Vec<u8>)> {
    ensure!(rest.len() == rest_len(header)?, "short frame");
    let (iv, sealed) = rest[..rest.len() - SUFFIX.len()].split_at(IV);
    let body = cipher(key)
        .decrypt(Nonce::from_slice(iv), Payload { msg: sealed, aad: &header[4..HEADER] })
        .map_err(|_| anyhow!("the plug's answer could not be read: its key is probably wrong"))?;
    Ok((u32::from_be_bytes(header[10..14].try_into().unwrap()), body))
}

/// The key of one connection: each side's random number combined, then
/// sealed with the plug's own key so only its holders can work it out.
fn session_key(key: &Key, mine: &[u8; 16], theirs: &[u8; 16]) -> Result<Key> {
    let mixed: [u8; 16] = std::array::from_fn(|i| mine[i] ^ theirs[i]);
    let sealed = cipher(key).encrypt(Nonce::from_slice(&mine[..IV]), mixed.as_slice()).map_err(|_| anyhow!("could not derive the session key"))?;
    Ok(sealed[..16].try_into().unwrap())
}

/// What a plug sent, without the result code and version it puts in front.
struct Answer {
    /// Zero when the plug did what it was asked.
    code: u32,
    body: Vec<u8>,
}

fn answer(plain: Vec<u8>) -> Answer {
    let (code, body) = match plain.split_first_chunk::<4>() {
        Some((code, body)) => (u32::from_be_bytes(*code), body),
        None => (0, plain.as_slice()),
    };
    let body = body.strip_prefix(VERSION.as_slice()).unwrap_or(body);
    Answer { code, body: body.to_vec() }
}

/// The first whole frame among the bytes received so far, taken out of them;
/// `None` while it is still arriving.
fn take_frame(buf: &mut Vec<u8>, key: &Key) -> Result<Option<(u32, Vec<u8>)>> {
    let Some(header) = buf.first_chunk::<HEADER>().copied() else { return Ok(None) };
    let total = HEADER + rest_len(&header)?;
    if buf.len() < total {
        return Ok(None);
    }
    let frame = open(key, &header, &buf[HEADER..total]);
    buf.drain(..total);
    frame.map(Some)
}

struct Session {
    s: TcpStream,
    key: Key,
    seq: u32,
    /// Bytes received that are not a whole frame yet.
    buf: Vec<u8>,
}

impl Session {
    async fn start(addr: &str, key: &Key) -> Result<Self> {
        let s = tokio::time::timeout(CONNECT, TcpStream::connect(addr)).await.map_err(|_| anyhow!("The plug did not answer at {addr}"))?.with_context(|| format!("The plug could not be reached at {addr}"))?;
        s.set_nodelay(true)?;
        let mut me = Self { s, key: *key, seq: 1, buf: Vec::new() };
        tokio::time::timeout(ANSWER, me.agree(key)).await.map_err(|_| anyhow!("The plug did not answer: its key is probably wrong"))??;
        Ok(me)
    }

    /// Each side sends a random number and proves, by signing the other's,
    /// that it holds the plug's key. A plug hangs up on a stranger.
    async fn agree(&mut self, key: &Key) -> Result<()> {
        let mut mine = [0u8; 16];
        OsRng.fill_bytes(&mut mine);
        self.send(SESSION_START, &mine).await?;
        let reply = self.expect(SESSION_REPLY).await.context("The plug hung up: its key is probably wrong")?;
        ensure!(reply.body.len() >= 48, "the plug's greeting was cut short");
        let theirs: [u8; 16] = reply.body[..16].try_into().unwrap();
        mac(key, &mine).verify_slice(&reply.body[16..48]).map_err(|_| anyhow!("That is not the plug this key belongs to"))?;
        self.send(SESSION_FINISH, &mac(key, &theirs).finalize().into_bytes()).await?;
        self.key = session_key(key, &mine, &theirs)?;
        Ok(())
    }

    async fn send(&mut self, cmd: u32, body: &[u8]) -> Result<()> {
        let mut iv = [0u8; IV];
        OsRng.fill_bytes(&mut iv);
        let frame = seal(&self.key, self.seq, cmd, body, iv)?;
        self.seq += 1;
        self.s.write_all(&frame).await?;
        Ok(())
    }

    /// The next answer to this command. A plug also reports changes nobody
    /// asked about, in between; those are passed over.
    /// The next frame. A wait for one may be given up (a plug that never
    /// acknowledges) and taken up again later, so what has arrived is kept
    /// here: half a frame dropped on the floor would garble all that follow.
    async fn recv(&mut self) -> Result<(u32, Vec<u8>)> {
        loop {
            if let Some(frame) = take_frame(&mut self.buf, &self.key)? {
                return Ok(frame);
            }
            let mut chunk = [0u8; 1024];
            let n = self.s.read(&mut chunk).await?;
            ensure!(n > 0, "the plug closed the connection");
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    async fn expect(&mut self, cmd: u32) -> Result<Answer> {
        loop {
            let (got, plain) = self.recv().await?;
            if got == cmd {
                return Ok(answer(plain));
            }
        }
    }

    async fn state(&mut self) -> Result<PlugState> {
        self.send(QUERY, b"{}").await?;
        let a = tokio::time::timeout(ANSWER, self.expect(QUERY)).await.map_err(|_| anyhow!("The plug stopped answering"))??;
        ensure!(a.code == 0, "the plug refused to report its state (code {})", a.code);
        PlugState::parse(&a.body)
    }

    async fn set(&mut self, dp: &str, value: Value) -> Result<PlugState> {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut body = VERSION.to_vec();
        body.extend_from_slice(json!({ "protocol": 5, "t": t, "data": { "dps": { dp: value } } }).to_string().as_bytes());
        self.send(CONTROL, &body).await?;
        // Not every firmware acknowledges, so silence is not a refusal; what counts is the state read back.
        if let Ok(a) = tokio::time::timeout(ACK, self.expect(CONTROL)).await {
            let a = a?;
            ensure!(a.code == 0, "The plug refused the change (code {})", a.code);
        }
        self.state().await
    }
}

/// What a plug is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlugState {
    pub on: bool,
    /// The delay last set for it to switch by itself, in seconds; zero when
    /// none is pending. A plug reports the delay it was given, not how much
    /// of it is left.
    pub countdown: u32,
    /// Power drawn through it, when it meters. Readings lag by some seconds.
    pub watts: Option<f32>,
}

impl PlugState {
    fn parse(body: &[u8]) -> Result<Self> {
        let v: Value = serde_json::from_slice(body).context("the plug's state could not be read")?;
        let dps = v.get("dps").or_else(|| v.get("data").and_then(|d| d.get("dps"))).ok_or_else(|| anyhow!("the plug reported no state"))?;
        Ok(Self {
            on: dps.get(DP_SWITCH).and_then(Value::as_bool).ok_or_else(|| anyhow!("That device is not a plug: it has no switch"))?,
            countdown: dps.get(DP_COUNTDOWN).and_then(Value::as_u64).unwrap_or(0) as u32,
            watts: dps.get(DP_POWER).and_then(Value::as_f64).map(|tenths| tenths as f32 / 10.0),
        })
    }
}

/// Longest delay a plug takes: a day.
pub const MAX_COUNTDOWN: u32 = 86_400;

#[derive(Clone)]
pub struct Plug {
    addr: String,
    key: Key,
}

impl Plug {
    /// `host` is the plug's address on the network, with or without a port;
    /// `key` is its local key, sixteen characters.
    pub fn new(host: &str, key: &str) -> Result<Self> {
        let host = host.trim();
        ensure!(!host.is_empty(), "The plug's address is missing");
        // A name or an IPv4 address, with or without a port; nothing that could be a URL or a path.
        let plain = host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'));
        ensure!(plain && host.matches(':').count() <= 1, "That does not look like a network address");
        let key = key.trim();
        ensure!(key.is_ascii(), "A plug's local key has only plain letters, digits and symbols");
        let key: Key = key.as_bytes().try_into().map_err(|_| anyhow!("A plug's local key has exactly 16 characters"))?;
        let addr = if host.contains(':') { host.to_owned() } else { format!("{host}:{PORT}") };
        Ok(Self { addr, key })
    }

    pub async fn state(&self) -> Result<PlugState> {
        Session::start(&self.addr, &self.key).await?.state().await
    }

    /// Have the plug flip its switch by itself after this many seconds; zero
    /// calls a pending one off. The plug counts on its own from here, with
    /// or without this computer.
    pub async fn set_countdown(&self, secs: u32) -> Result<()> {
        if secs > MAX_COUNTDOWN {
            bail!("A plug's timer goes up to a day");
        }
        let now = Session::start(&self.addr, &self.key).await?.set(DP_COUNTDOWN, secs.into()).await?;
        ensure!(now.countdown == secs, "The plug did not take the timer");
        Ok(())
    }
}

/// Every other address on the networks this computer is on, taking each as
/// the usual home network of 254 hosts.
pub fn neighbours(own: &[std::net::Ipv4Addr]) -> Vec<std::net::Ipv4Addr> {
    let mut nets: Vec<[u8; 3]> = own.iter().filter(|a| a.is_private()).map(|a| [a.octets()[0], a.octets()[1], a.octets()[2]]).collect();
    nets.sort();
    nets.dedup();
    nets.into_iter().flat_map(|[a, b, c]| (1..255).map(move |d| std::net::Ipv4Addr::new(a, b, c, d))).filter(|a| !own.contains(a)).collect()
}

/// The plug this key belongs to, among these addresses (`host:port`). Plugs
/// announce themselves on the network too, but a firewall often drops that;
/// knocking on each door and trying the key does not depend on it.
pub async fn find(addrs: Vec<String>, key: &str) -> Result<(String, Plug)> {
    Plug::new("-", key)?;
    let mut knocks = tokio::task::JoinSet::new();
    for addr in addrs {
        knocks.spawn(async move { matches!(tokio::time::timeout(Duration::from_millis(800), TcpStream::connect(&addr)).await, Ok(Ok(_))).then_some(addr) });
    }
    let mut open = Vec::new();
    while let Some(r) = knocks.join_next().await {
        open.extend(r.ok().flatten());
    }
    open.sort();
    for addr in open {
        let plug = Plug::new(&addr, key)?;
        if plug.state().await.is_ok() {
            return Ok((addr, plug));
        }
    }
    bail!("No plug on this network answers to that key. Check that it is on the same network as this computer, or type its address.")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    pub(crate) const KEY: &Key = b"0123456789abcdef";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn split(frame: &[u8]) -> ([u8; HEADER], &[u8]) {
        (frame[..HEADER].try_into().unwrap(), &frame[HEADER..])
    }

    // The three vectors below come from tinytuya, the reference client, with this key and IV.

    #[test]
    fn seals_as_the_reference_does() {
        let frame = seal(KEY, 7, QUERY, b"{}", *b"abcdefghijkl").unwrap();
        assert_eq!(frame, hex("00006699000000000007000000100000001e6162636465666768696a6b6c9eaf2bf09c6673b0087216746b9258e3a14f00009966"));
    }

    #[test]
    fn opens_what_a_plug_reports() {
        let frame = hex(
            "00006699000000000009000000080000005b6162636465666768696a6b6ce5d27cb5608b6ca55930b43af63edf090a15ec9c1a8b413df093ae173e757285303c2bb30482fd6ae1947c3e0dec715cef84770374b0e5c58907579b49482302e921aafa7ab353f3b471731f9aba6000009966",
        );
        let (header, rest) = split(&frame);
        let (cmd, plain) = open(KEY, &header, rest).unwrap();
        let a = answer(plain);
        assert_eq!((cmd, a.code), (8, 0));
        assert_eq!(a.body, br#"{"protocol":4,"t":1,"data":{"dps":{"9":20}}}"#);
        assert_eq!(PlugState::parse(&a.body).unwrap_err().to_string(), "That device is not a plug: it has no switch");
    }

    #[test]
    fn derives_the_session_key_as_the_reference_does() {
        let mine: [u8; 16] = std::array::from_fn(|i| i as u8);
        let theirs: [u8; 16] = std::array::from_fn(|i| 100 + i as u8);
        assert_eq!(session_key(KEY, &mine, &theirs).unwrap().to_vec(), hex("995ba0797218575a8c42637ebce7cc51"));
    }

    #[test]
    fn refuses_frames_that_were_touched_or_are_not_tuya() {
        let frame = seal(KEY, 1, QUERY, b"{}", [7; IV]).unwrap();
        let (header, rest) = split(&frame);
        assert!(open(KEY, &header, rest).is_ok());
        // Another key, a changed body, and a changed header (it is in the clear, but vouched for).
        assert!(open(b"another key 1234", &header, rest).is_err());
        let mut body = rest.to_vec();
        body[IV] ^= 1;
        assert!(open(KEY, &header, &body).is_err());
        let mut other = header;
        other[13] = CONTROL as u8;
        assert!(open(KEY, &other, rest).is_err());
        assert!(open(KEY, &header, &rest[..rest.len() - 1]).is_err());

        let mut old = header;
        old[2..4].copy_from_slice(&[0x55, 0xaa]);
        assert_eq!(rest_len(&old).unwrap_err().to_string(), "that is not a Tuya 3.5 device");
        let mut huge = header;
        huge[14..18].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(rest_len(&huge).is_err());
        let mut tiny = header;
        tiny[14..18].copy_from_slice(&3u32.to_be_bytes());
        assert!(rest_len(&tiny).is_err());
    }

    #[test]
    fn reads_state() {
        let s = PlugState::parse(br#"{"dps":{"1":true,"9":0,"19":1647,"20":1259}}"#).unwrap();
        assert_eq!(s, PlugState { on: true, countdown: 0, watts: Some(164.7) });
        // A plug that does not meter, reporting through the wrapped form.
        let s = PlugState::parse(br#"{"protocol":4,"data":{"dps":{"1":false,"9":300}}}"#).unwrap();
        assert_eq!(s, PlugState { on: false, countdown: 300, watts: None });
        assert!(PlugState::parse(b"{}").is_err());
        assert!(PlugState::parse(b"not json").is_err());
    }

    #[test]
    fn checks_the_address_and_key() {
        assert_eq!(Plug::new("192.168.1.22", "0123456789abcdef").unwrap().addr, "192.168.1.22:6668");
        assert_eq!(Plug::new(" 192.168.1.22:7000 ", " 0123456789abcdef ").unwrap().addr, "192.168.1.22:7000");
        assert_eq!(Plug::new("plug.lan", "0123456789abcdef").unwrap().addr, "plug.lan:6668");
        assert!(Plug::new("", "0123456789abcdef").is_err());
        for host in ["http://192.168.1.22", "192.168.1.22/x", "a b", "[::1]:6668", "::1", "host:1:2", "plug\u{0}.lan"] {
            assert!(Plug::new(host, "0123456789abcdef").is_err(), "{host}");
        }
        for key in ["", "short", "0123456789abcdefg", "0123456789abcdé"] {
            assert!(Plug::new("192.168.1.22", key).is_err(), "{key}");
        }
    }

    async fn read_frame(s: &mut TcpStream, key: &Key) -> Result<(u32, Vec<u8>)> {
        let mut header = [0u8; HEADER];
        s.read_exact(&mut header).await?;
        let mut rest = vec![0u8; rest_len(&header)?];
        s.read_exact(&mut rest).await?;
        open(key, &header, &rest)
    }

    #[test]
    fn takes_frames_out_of_whatever_has_arrived() {
        let one = seal(KEY, 1, QUERY, b"{}", [1; IV]).unwrap();
        let two = seal(KEY, 2, CONTROL, b"second", [2; IV]).unwrap();
        // Nothing, part of a header, and a whole header with part of a body: all "not yet".
        let mut buf;
        for upto in [0, 5, HEADER, one.len() - 1] {
            buf = one[..upto].to_vec();
            assert_eq!(take_frame(&mut buf, KEY).unwrap(), None);
            assert_eq!(buf.len(), upto);
        }
        // One and a half frames: the first comes out, the half waits for its rest.
        buf = [one.as_slice(), &two[..10]].concat();
        assert_eq!(take_frame(&mut buf, KEY).unwrap(), Some((QUERY, b"{}".to_vec())));
        assert_eq!(take_frame(&mut buf, KEY).unwrap(), None);
        buf.extend_from_slice(&two[10..]);
        assert_eq!(take_frame(&mut buf, KEY).unwrap(), Some((CONTROL, b"second".to_vec())));
        assert!(buf.is_empty());
        // A frame that cannot be opened is an error, and does not jam the ones behind it.
        buf = [seal(b"another key 1234", 1, QUERY, b"{}", [1; IV]).unwrap(), two.clone()].concat();
        assert!(take_frame(&mut buf, KEY).is_err());
        assert_eq!(take_frame(&mut buf, KEY).unwrap(), Some((CONTROL, b"second".to_vec())));
    }

    /// What a made-up plug does, and what it was last told.
    #[derive(Default)]
    pub(crate) struct Device {
        pub on: bool,
        pub countdown: u32,
        /// Acknowledges changes and reports them, but does not make them.
        pub deaf: bool,
        /// Says nothing when changed, as some firmware does.
        pub silent: bool,
        /// Sends its acknowledgement in two pieces, with a pause longer than this client waits for one.
        pub slow: bool,
    }

    /// A plug on a local socket, speaking its side of the protocol.
    pub(crate) async fn device(key: Key, dev: Arc<Mutex<Device>>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let dev = dev.clone();
                tokio::spawn(async move {
                    let mut seq = 100;
                    let mut now = key;
                    let mine = [9u8; 16];
                    let mut next = None;
                    // A frame this plug cannot open, or a failed proof, ends the connection.
                    while let Ok((cmd, body)) = read_frame(&mut s, &now).await {
                        let mut out: Vec<(u32, Vec<u8>)> = Vec::new();
                        let report = |d: &Device| json!({ "dps": { "1": d.on, "9": d.countdown, "19": 522 } }).to_string().into_bytes();
                        match cmd {
                            SESSION_START => {
                                let theirs: [u8; 16] = body[..16].try_into().unwrap();
                                next = Some(session_key(&key, &theirs, &mine).unwrap());
                                out.push((SESSION_REPLY, [mine.as_slice(), &mac(&key, &theirs).finalize().into_bytes()].concat()));
                            }
                            SESSION_FINISH => {
                                if mac(&key, &mine).verify_slice(&body).is_err() {
                                    return;
                                }
                                now = next.take().unwrap();
                                continue;
                            }
                            QUERY => out.push((QUERY, report(&dev.lock().unwrap()))),
                            CONTROL => {
                                let v: Value = serde_json::from_slice(body.strip_prefix(VERSION.as_slice()).unwrap()).unwrap();
                                assert_eq!(v["protocol"], 5);
                                let mut d = dev.lock().unwrap();
                                if !d.deaf {
                                    if let Some(on) = v["data"]["dps"]["1"].as_bool() {
                                        d.on = on;
                                    }
                                    if let Some(c) = v["data"]["dps"]["9"].as_u64() {
                                        d.countdown = c as u32;
                                    }
                                }
                                if !d.silent {
                                    // An unasked report first, then the acknowledgement.
                                    out.push((8, [VERSION.as_slice(), &report(&d)].concat()));
                                    out.push((CONTROL, Vec::new()));
                                }
                            }
                            _ => {}
                        }
                        for (cmd, body) in out {
                            // A plug puts a result code in front of everything it sends.
                            let frame = seal(&now, seq, cmd, &[&[0u8; 4], body.as_slice()].concat(), [seq as u8; IV]).unwrap();
                            seq += 1;
                            let slow = dev.lock().unwrap().slow && cmd == CONTROL;
                            let (head, tail) = frame.split_at(if slow { 7 } else { frame.len() });
                            if s.write_all(head).await.is_err() {
                                return;
                            }
                            if slow {
                                tokio::time::sleep(ACK + Duration::from_millis(300)).await;
                            }
                            if s.write_all(tail).await.is_err() {
                                return;
                            }
                        }
                    }
                });
            }
        });
        addr
    }

    pub(crate) async fn plug(dev: Device) -> (Plug, Arc<Mutex<Device>>) {
        let dev = Arc::new(Mutex::new(dev));
        let addr = device(*KEY, dev.clone()).await;
        (Plug::new(&addr, std::str::from_utf8(KEY).unwrap()).unwrap(), dev)
    }

    #[tokio::test]
    async fn reads_a_plug() {
        let (p, dev) = plug(Device { on: true, ..Default::default() }).await;
        assert_eq!(p.state().await.unwrap(), PlugState { on: true, countdown: 0, watts: Some(52.2) });
        dev.lock().unwrap().on = false;
        assert!(!p.state().await.unwrap().on);
    }

    #[tokio::test]
    async fn sets_and_calls_off_the_timer() {
        let (p, dev) = plug(Device { on: true, ..Default::default() }).await;
        p.set_countdown(300).await.unwrap();
        assert_eq!(dev.lock().unwrap().countdown, 300);
        assert_eq!(p.state().await.unwrap().countdown, 300);
        p.set_countdown(0).await.unwrap();
        assert_eq!(dev.lock().unwrap().countdown, 0);
        // More than a day is refused before anything is sent.
        assert!(p.set_countdown(MAX_COUNTDOWN + 1).await.is_err());
        assert_eq!(dev.lock().unwrap().countdown, 0);
    }

    #[tokio::test]
    async fn a_change_the_plug_did_not_make_is_an_error() {
        let (p, _) = plug(Device { on: true, deaf: true, ..Default::default() }).await;
        assert_eq!(p.set_countdown(300).await.unwrap_err().to_string(), "The plug did not take the timer");
    }

    #[tokio::test]
    async fn a_plug_that_does_not_acknowledge_still_works() {
        let (p, dev) = plug(Device { on: true, silent: true, ..Default::default() }).await;
        p.set_countdown(120).await.unwrap();
        assert_eq!(dev.lock().unwrap().countdown, 120);
    }

    #[tokio::test]
    async fn an_acknowledgement_that_arrives_in_pieces_does_not_garble_what_follows() {
        // Half of it arrives, the wait for it is given up, and the rest comes while the state is being asked for.
        let (p, dev) = plug(Device { on: true, slow: true, ..Default::default() }).await;
        p.set_countdown(90).await.unwrap();
        assert_eq!(dev.lock().unwrap().countdown, 90);
    }

    #[tokio::test]
    async fn the_wrong_key_is_told_apart_from_no_plug() {
        let dev = Arc::new(Mutex::new(Device::default()));
        let addr = device(*KEY, dev).await;
        let e = Plug::new(&addr, "another key 1234").unwrap().state().await.unwrap_err();
        assert!(format!("{e:#}").contains("key is probably wrong"), "{e:#}");

        // Nothing listening there at all.
        let free = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().to_string();
        let e = Plug::new(&free, "0123456789abcdef").unwrap().state().await.unwrap_err();
        assert!(format!("{e:#}").contains("could not be reached") || format!("{e:#}").contains("did not answer at"), "{e:#}");
    }

    #[test]
    fn looks_around_its_own_networks() {
        use std::net::Ipv4Addr;
        let own = [Ipv4Addr::new(192, 168, 15, 11), Ipv4Addr::new(192, 168, 15, 12), Ipv4Addr::new(10, 0, 0, 5), Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::LOCALHOST];
        let n = neighbours(&own);
        // Two home networks, 254 hosts each, minus this computer's own three addresses on them.
        assert_eq!(n.len(), 2 * 254 - 3);
        assert!(n.contains(&Ipv4Addr::new(192, 168, 15, 22)) && n.contains(&Ipv4Addr::new(10, 0, 0, 254)));
        assert!(!n.contains(&Ipv4Addr::new(192, 168, 15, 11)) && !n.contains(&Ipv4Addr::new(192, 168, 15, 255)) && !n.contains(&Ipv4Addr::new(192, 168, 15, 0)));
        // The open internet and this computer itself are never knocked on.
        assert!(n.iter().all(|a| a.octets()[0] != 8 && !a.is_loopback()));
        assert!(neighbours(&[]).is_empty());
    }

    #[tokio::test]
    async fn finds_the_plug_the_key_belongs_to() {
        let mine = device(*KEY, Arc::new(Mutex::new(Device { on: true, ..Default::default() }))).await;
        let other = device(*b"someone elses 16", Arc::new(Mutex::new(Device::default()))).await;
        // Something that is open but is no plug, and somewhere nothing listens.
        let web = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().to_string();
        let addrs = vec![closed, web.local_addr().unwrap().to_string(), other.clone(), mine.clone()];
        let key = std::str::from_utf8(KEY).unwrap();
        let (addr, plug) = find(addrs.clone(), key).await.unwrap();
        assert_eq!(addr, mine);
        assert!(plug.state().await.unwrap().on);
        // A key no plug here answers to, and a key that is not one.
        assert!(find(addrs.clone(), "nobody has it 16").await.err().unwrap().to_string().contains("No plug on this network"));
        assert!(find(addrs, "short").await.is_err());
    }

    /// Reads a real plug, changing nothing: set `SWITCHBOARD_TEST_PLUG` to
    /// its address and `SWITCHBOARD_TEST_PLUG_KEY` to its local key, then
    /// `cargo test --bins -- --ignored real_plug`.
    #[tokio::test]
    #[ignore = "needs a real plug on the network"]
    async fn real_plug_reports_its_state() {
        let p = Plug::new(&std::env::var("SWITCHBOARD_TEST_PLUG").unwrap(), &std::env::var("SWITCHBOARD_TEST_PLUG_KEY").unwrap()).unwrap();
        let s = p.state().await.unwrap();
        println!("{s:?}");
        // Setting the timer to what it already is exercises a write without changing anything.
        p.set_countdown(s.countdown).await.unwrap();
    }
}
