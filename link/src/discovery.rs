//! Descubrimiento en la red local: cada equipo anuncia por UDP (difusión, puerto 47801) su nombre,
//! sistema, puerto y monitores. Los dispositivos agregados por IP se consultan directamente, por si
//! la red bloquea la difusión.
use serde_json::{Value, json};
use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};

pub const PORT: u16 = 47801;

#[derive(Clone)]
pub struct Beacon {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub port: u16,
    /// (ancho, alto) de cada monitor, de izquierda a derecha.
    pub displays: Vec<(u32, u32)>,
    pub sharing: bool,
    /// Puede crear pantallas virtuales para quien lo ve (macOS).
    pub vd: bool,
}

pub enum Packet {
    Beacon(Beacon),
    /// Petición de anuncio (se responde directamente a quien pregunta).
    Query,
}

/// Socket UDP compartible: varias instancias en el mismo equipo (pruebas) reciben la difusión.
pub fn open() -> io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(unix)]
    s.set_reuse_port(true)?;
    s.set_broadcast(true)?;
    s.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT)).into())?;
    Ok(s.into())
}

/// Dirección de difusión general y la de cada red IPv4 del equipo.
pub fn broadcast_targets() -> Vec<SocketAddr> {
    let mut v = vec![SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, PORT))];
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for i in ifs {
            if let if_addrs::IfAddr::V4(a) = i.addr {
                if a.ip.is_loopback() { continue; }
                if let Some(b) = a.broadcast {
                    let t = SocketAddr::V4(SocketAddrV4::new(b, PORT));
                    if !v.contains(&t) { v.push(t); }
                }
            }
        }
    }
    v
}

pub fn encode(b: &Beacon) -> Vec<u8> {
    json!({
        "kast": 1, "id": b.id, "name": b.name, "platform": b.platform, "port": b.port, "sharing": b.sharing, "vd": b.vd,
        "displays": b.displays.iter().map(|(w, h)| json!([w, h])).collect::<Vec<_>>(),
    })
    .to_string()
    .into_bytes()
}

pub fn query() -> Vec<u8> { br#"{"kast":1,"q":1}"#.to_vec() }

pub fn decode(data: &[u8]) -> Option<Packet> {
    let v: Value = serde_json::from_slice(data).ok()?;
    if v.get("kast")?.as_u64()? != 1 { return None; }
    if v.get("q").is_some() { return Some(Packet::Query); }
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(|x| x.chars().take(64).collect::<String>());
    Some(Packet::Beacon(Beacon {
        id: s("id")?,
        name: s("name").unwrap_or_default(),
        platform: s("platform").unwrap_or_default(),
        port: v.get("port")?.as_u64()? as u16,
        sharing: v.get("sharing").and_then(Value::as_bool).unwrap_or(true),
        vd: v.get("vd").and_then(Value::as_bool).unwrap_or(false),
        displays: v.get("displays").and_then(Value::as_array).map(|a| {
            a.iter().take(16).filter_map(|d| Some((d.get(0)?.as_u64()? as u32, d.get(1)?.as_u64()? as u32))).collect()
        }).unwrap_or_default(),
    }))
}
