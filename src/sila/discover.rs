//! DNS-SD advertisement for `_sila._tcp.local.`.
//!
//! The listener uses `SO_REUSEADDR` and not `SO_REUSEPORT`. The official C# client binds
//! port 5353 the same way, and Linux delivers multicast to every socket in that set.

use std::collections::BTreeMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

use crate::error::A2aLabError;
use crate::sila::cert::SilaCertificate;
use crate::sila::identity::SilaIdentity;

const SERVICE: &str = "_sila._tcp.local";
const MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;
const CLASS_IN: u16 = 1;
const CLASS_UNIQUE: u16 = 0x8001;
const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
const TYPE_TXT: u16 = 16;
const TYPE_SRV: u16 = 33;
const TYPE_ANY: u16 = 255;

pub(crate) struct Announcer {
    shutdown: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Announcer {
    pub(crate) fn start(
        address: SocketAddr,
        identity: &SilaIdentity,
        certificate: Option<&SilaCertificate>,
    ) -> Result<Self, A2aLabError> {
        let advertisement = Advertisement::new(address, identity, certificate)?;
        let socket = mdns_socket(advertisement.ip)?;
        let (shutdown, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("sila-mdns".to_owned())
            .spawn(move || serve(&advertisement, &receiver, &socket))
            .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
        Ok(Self {
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    pub(crate) fn refresh(
        &mut self,
        address: SocketAddr,
        identity: &SilaIdentity,
        certificate: Option<&SilaCertificate>,
    ) -> Result<(), A2aLabError> {
        self.stop();
        let mut started = Self::start(address, identity, certificate)?;
        self.shutdown = started.shutdown.take();
        self.thread = started.thread.take();
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Announcer {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Advertisement {
    instance: String,
    host: String,
    port: u16,
    ip: Ipv4Addr,
    txt: Vec<String>,
}

impl Advertisement {
    fn new(
        address: SocketAddr,
        identity: &SilaIdentity,
        certificate: Option<&SilaCertificate>,
    ) -> Result<Self, A2aLabError> {
        let ip = reachable_ip(address)
            .parse()
            .map_err(|_| A2aLabError::protocol("SiLA discovery needs an IPv4 address"))?;
        Ok(Self {
            instance: format!("{}.{SERVICE}", identity.server_uuid),
            host: format!("{}.local", identity.server_uuid),
            port: address.port(),
            ip,
            txt: txt_fields(identity, certificate),
        })
    }
}

pub(crate) fn properties(
    identity: &SilaIdentity,
    certificate: Option<&SilaCertificate>,
) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::from([
        ("version".to_owned(), "1.1".to_owned()),
        ("server_name".to_owned(), identity.server_name.clone()),
        ("server_type".to_owned(), identity.server_type.clone()),
        (
            "description".to_owned(),
            truncate(&identity.description, 220),
        ),
        ("vendor_url".to_owned(), identity.vendor_url.clone()),
    ]);
    if let Some(certificate) = certificate {
        for (index, line) in certificate
            .ca_pem
            .lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
        {
            properties.insert(format!("ca{index}"), line.to_owned());
        }
    }
    properties
}

fn txt_fields(identity: &SilaIdentity, certificate: Option<&SilaCertificate>) -> Vec<String> {
    properties(identity, certificate)
        .into_iter()
        .filter_map(|(key, value)| txt_string(&key, &value))
        .collect()
}

fn txt_string(key: &str, value: &str) -> Option<String> {
    let prefix = format!("{key}=");
    if prefix.len() >= 255 {
        return None;
    }
    let budget = 255 - prefix.len();
    Some(format!("{prefix}{}", truncate_bytes(value, budget)))
}

fn truncate_bytes(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn goodbye(advertisement: &Advertisement) -> Vec<u8> {
    message(
        &[
            record(
                SERVICE,
                TYPE_PTR,
                CLASS_IN,
                0,
                encode_name(&advertisement.instance),
            ),
            record_with(
                &advertisement.instance,
                TYPE_SRV,
                srv_body(advertisement),
                0,
            ),
            record_with(
                &advertisement.instance,
                TYPE_TXT,
                txt_body(advertisement),
                0,
            ),
            record_with(
                &advertisement.host,
                TYPE_A,
                advertisement.ip.octets().to_vec(),
                0,
            ),
        ],
        &[],
    )
}

fn serve(advertisement: &Advertisement, shutdown: &Receiver<()>, socket: &UdpSocket) {
    let destination = SocketAddr::from((MULTICAST, MDNS_PORT));
    let _ = socket.send_to(&ptr_response(advertisement, 4_500), destination);
    let mut buffer = [0_u8; 2048];
    loop {
        if shutdown.try_recv().is_ok() {
            let _ = socket.send_to(&goodbye(advertisement), destination);
            break;
        }
        match socket.recv_from(&mut buffer) {
            Ok((size, _)) => {
                if let Some(response) = answer_query(advertisement, &buffer[..size]) {
                    let _ = socket.send_to(&response, destination);
                }
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }
}

fn mdns_socket(interface: Ipv4Addr) -> Result<UdpSocket, A2aLabError> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
    socket
        .set_reuse_address(true)
        .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
    socket
        .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, MDNS_PORT)).into())
        .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
    socket
        .join_multicast_v4(&MULTICAST, &interface)
        .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
    socket
        .set_multicast_if_v4(&interface)
        .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
    socket
        .set_multicast_loop_v4(true)
        .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
    socket
        .set_multicast_ttl_v4(255)
        .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
    let socket = UdpSocket::from(socket);
    socket
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|error| A2aLabError::unavailable(error.to_string()))?;
    Ok(socket)
}

fn answer_query(advertisement: &Advertisement, query: &[u8]) -> Option<Vec<u8>> {
    let questions = questions(query)?;
    let service = |name: &str| normalize(name) == SERVICE;
    let instance = |name: &str| normalize(name) == normalize(&advertisement.instance);
    let host = |name: &str| normalize(name) == normalize(&advertisement.host);
    if questions
        .iter()
        .any(|(name, typ)| service(name) && (*typ == TYPE_PTR || *typ == TYPE_ANY))
    {
        return Some(ptr_response(advertisement, 4_500));
    }
    if questions.iter().any(|(name, typ)| {
        instance(name) && (*typ == TYPE_SRV || *typ == TYPE_TXT || *typ == TYPE_ANY)
    }) {
        return Some(srv_response(advertisement));
    }
    if questions
        .iter()
        .any(|(name, typ)| host(name) && (*typ == TYPE_A || *typ == TYPE_ANY))
    {
        return Some(address_response(advertisement));
    }
    None
}

fn ptr_response(advertisement: &Advertisement, ttl: u32) -> Vec<u8> {
    message(
        &[record(
            SERVICE,
            TYPE_PTR,
            CLASS_IN,
            ttl,
            encode_name(&advertisement.instance),
        )],
        &[
            srv_record(advertisement),
            txt_record(advertisement),
            address_record(advertisement),
        ],
    )
}

fn srv_response(advertisement: &Advertisement) -> Vec<u8> {
    message(
        &[srv_record(advertisement)],
        &[address_record(advertisement), txt_record(advertisement)],
    )
}

fn address_response(advertisement: &Advertisement) -> Vec<u8> {
    message(&[address_record(advertisement)], &[])
}

fn srv_record(advertisement: &Advertisement) -> Record {
    record_with(
        &advertisement.instance,
        TYPE_SRV,
        srv_body(advertisement),
        120,
    )
}

fn srv_body(advertisement: &Advertisement) -> Vec<u8> {
    let mut rdata = vec![0, 0, 0, 0];
    rdata.extend(advertisement.port.to_be_bytes());
    rdata.extend(encode_name(&advertisement.host));
    rdata
}

fn txt_record(advertisement: &Advertisement) -> Record {
    record_with(
        &advertisement.instance,
        TYPE_TXT,
        txt_body(advertisement),
        4_500,
    )
}

fn txt_body(advertisement: &Advertisement) -> Vec<u8> {
    let mut rdata = Vec::new();
    for field in &advertisement.txt {
        let bytes = field.as_bytes();
        let Ok(len) = u8::try_from(bytes.len()) else {
            continue;
        };
        rdata.push(len);
        rdata.extend(bytes);
    }
    rdata
}

fn record_with(name: &str, typ: u16, rdata: Vec<u8>, ttl: u32) -> Record {
    record(name, typ, CLASS_UNIQUE, ttl, rdata)
}

fn address_record(advertisement: &Advertisement) -> Record {
    record(
        &advertisement.host,
        TYPE_A,
        CLASS_UNIQUE,
        120,
        advertisement.ip.octets().to_vec(),
    )
}

struct Record {
    name: String,
    typ: u16,
    class: u16,
    ttl: u32,
    rdata: Vec<u8>,
}

fn record(name: &str, typ: u16, class: u16, ttl: u32, rdata: Vec<u8>) -> Record {
    Record {
        name: name.to_owned(),
        typ,
        class,
        ttl,
        rdata,
    }
}

fn message(answers: &[Record], additional: &[Record]) -> Vec<u8> {
    let mut packet = vec![0_u8; 12];
    packet[2] = 0x84;
    let answer_count = u16::try_from(answers.len()).unwrap_or(0);
    let additional_count = u16::try_from(additional.len()).unwrap_or(0);
    packet[6..8].copy_from_slice(&answer_count.to_be_bytes());
    packet[10..12].copy_from_slice(&additional_count.to_be_bytes());
    for item in answers.iter().chain(additional) {
        packet.extend(encode_name(&item.name));
        packet.extend(item.typ.to_be_bytes());
        packet.extend(item.class.to_be_bytes());
        packet.extend(item.ttl.to_be_bytes());
        let len = u16::try_from(item.rdata.len()).unwrap_or(0);
        packet.extend(len.to_be_bytes());
        packet.extend(&item.rdata);
    }
    packet
}

fn encode_name(name: &str) -> Vec<u8> {
    let mut encoded = Vec::new();
    for label in name.split('.').filter(|label| !label.is_empty()) {
        let Ok(len) = u8::try_from(label.len()) else {
            continue;
        };
        if len > 63 {
            continue;
        }
        encoded.push(len);
        encoded.extend(label.as_bytes());
    }
    encoded.push(0);
    encoded
}

fn questions(packet: &[u8]) -> Option<Vec<(String, u16)>> {
    if packet.len() < 12 || packet[2] & 0x80 != 0 {
        return None;
    }
    let count = u16::from_be_bytes([packet[4], packet[5]]);
    let mut pos = 12;
    let mut parsed = Vec::new();
    for _ in 0..count {
        let (name, next) = decode_name(packet, pos)?;
        if next + 4 > packet.len() {
            return None;
        }
        let typ = u16::from_be_bytes([packet[next], packet[next + 1]]);
        pos = next + 4;
        parsed.push((name, typ));
    }
    Some(parsed)
}

fn decode_name(packet: &[u8], offset: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut pos = offset;
    let mut end = None;
    for _ in 0..16 {
        if pos >= packet.len() {
            return None;
        }
        let len = packet[pos];
        if len == 0 {
            if end.is_none() {
                end = Some(pos + 1);
            }
            return Some((labels.join("."), end?));
        }
        if len & 0xC0 == 0xC0 {
            if pos + 1 >= packet.len() {
                return None;
            }
            if end.is_none() {
                end = Some(pos + 2);
            }
            pos = (usize::from(len & 0x3F) << 8) | usize::from(packet[pos + 1]);
            continue;
        }
        let start = pos + 1;
        let stop = start + usize::from(len);
        if stop > packet.len() {
            return None;
        }
        labels.push(std::str::from_utf8(&packet[start..stop]).ok()?.to_owned());
        pos = stop;
    }
    None
}

fn normalize(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn reachable_ip(address: SocketAddr) -> String {
    if !address.ip().is_unspecified()
        && !address.ip().is_loopback()
        && let std::net::IpAddr::V4(ip) = address.ip()
    {
        return ip.to_string();
    }
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0")
        && socket.connect("8.8.8.8:80").is_ok()
        && let Ok(local) = socket.local_addr()
        && let std::net::IpAddr::V4(ip) = local.ip()
        && !ip.is_unspecified()
    {
        return ip.to_string();
    }
    "127.0.0.1".to_owned()
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut shortened: String = value.chars().take(limit.saturating_sub(4)).collect();
    shortened.push_str(" ...");
    shortened
}

/// Shared discovery handle so a rename can refresh TXT data.
pub(crate) type SharedAnnouncer = std::sync::Arc<std::sync::Mutex<Option<Announcer>>>;

#[cfg(test)]
mod tests {
    use super::{Advertisement, answer_query, encode_name, goodbye, properties, txt_string};
    use crate::sila::identity::SilaIdentity;
    use std::net::Ipv4Addr;

    #[test]
    fn advertises_protocol_version_and_identity() {
        let identity = SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap();
        let fields = properties(&identity, None);
        assert_eq!(fields.get("version").map(String::as_str), Some("1.1"));
        assert_eq!(
            fields.get("server_type").map(String::as_str),
            Some("LabDevKit")
        );
    }

    #[test]
    fn srv_answer_carries_the_address_and_txt() {
        let advertisement = sample();
        let mut query = vec![0_u8; 12];
        query[5] = 1;
        query.extend(encode_name(&advertisement.instance));
        query.extend(33_u16.to_be_bytes());
        query.extend(1_u16.to_be_bytes());
        let response = answer_query(&advertisement, &query).unwrap();
        assert_eq!(response[2], 0x84);
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
        assert_eq!(u16::from_be_bytes([response[10], response[11]]), 2);
        assert!(
            response
                .windows(4)
                .any(|bytes| bytes == advertisement.ip.octets())
        );
        assert!(response.windows(9).any(|bytes| bytes == b"version=1"));
    }

    #[test]
    fn goodbye_records_use_a_zero_ttl() {
        let packet = goodbye(&sample());
        assert_eq!(u16::from_be_bytes([packet[6], packet[7]]), 4);
        let mut offset = 12;
        for _ in 0..4 {
            offset = skip_name(&packet, offset);
            offset += 4;
            assert_eq!(&packet[offset..offset + 4], &[0, 0, 0, 0]);
            offset += 4;
            let len = usize::from(u16::from_be_bytes([packet[offset], packet[offset + 1]]));
            offset += 2 + len;
        }
        assert_eq!(offset, packet.len());
    }

    #[test]
    fn txt_values_stay_within_a_single_character_string() {
        let text = txt_string("name", &"é".repeat(300)).unwrap();
        assert!(text.len() <= 255);
        assert!(text.is_char_boundary(text.len()));
    }

    fn skip_name(packet: &[u8], mut offset: usize) -> usize {
        loop {
            let len = usize::from(packet[offset]);
            offset += 1;
            if len == 0 {
                break;
            }
            offset += len;
        }
        offset
    }

    fn sample() -> Advertisement {
        let identity = SilaIdentity::lab_dev_kit("11111111-1111-1111-1111-111111111111").unwrap();
        Advertisement {
            instance: format!("{}.{}", identity.server_uuid, super::SERVICE),
            host: format!("{}.local", identity.server_uuid),
            port: 50052,
            ip: Ipv4Addr::new(192, 168, 5, 15),
            txt: super::txt_fields(&identity, None),
        }
    }
}
