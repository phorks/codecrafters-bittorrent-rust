use core::str;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{self, Map};
use sha1::{Digest, Sha1};
use std::{
    env, fs,
    io::{Cursor, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    str::FromStr,
};

const PEER_ID: &str = "00112233445566778899";
const PROTOCOL_STRING: &str = "BitTorrent protocol";
const BLOCK_SIZE: u32 = 1 << 14;

// Available if you need it!
// use serde_bencode

struct Decode {
    length: usize,
    value: serde_json::Value,
}

impl Decode {
    fn new(length: usize, value: serde_json::Value) -> Self {
        Self { length, value }
    }
}

#[derive(Serialize, Deserialize)]
struct TorrentFile {
    announce: String,
    info: TorrentFileInfo,
}

impl TorrentFile {
    fn from_file(path: &str) -> Self {
        let bytes = fs::read(path).unwrap();
        return serde_bencode::from_bytes(&bytes).unwrap();
    }

    fn create_peer(&self, addr: SocketAddrV4) -> Peer {
        Peer {
            addr: addr,
            file: self,
        }
    }

    fn find_peers(&self) -> impl Iterator<Item = Peer> {
        let req = TrackerRequest {
            info_hash: self.info.hash(),
            peer_id: PEER_ID.into(),
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: self.info.length,
            compact: true,
        };

        let url = req.create_url(&self.announce);

        let client = reqwest::blocking::Client::new();
        let http_response = client.get(url).send().unwrap();

        let resp: TrackerResponse =
            serde_bencode::from_bytes(&http_response.bytes().unwrap()).unwrap();

        resp.get_peers()
            .into_iter()
            .map(move |addr| Peer { addr, file: &self })
    }
}

#[derive(Serialize, Deserialize)]
struct TorrentFileInfo {
    length: usize,
    name: String,
    #[serde(rename = "piece length")]
    plength: usize,
    #[serde(with = "serde_bytes")]
    pieces: Vec<u8>,
}

impl TorrentFileInfo {
    fn get_pieces(&self) -> impl Iterator<Item = &[u8]> {
        return self.pieces.chunks(20);
    }

    fn hash(&self) -> [u8; 20] {
        let data = &serde_bencode::to_bytes(&self).unwrap();
        let mut hasher = Sha1::new();
        hasher.update(data);
        hasher.finalize().into()
    }
}

#[derive(Serialize)]
struct TrackerRequest {
    #[serde(skip_serializing)]
    info_hash: [u8; 20],
    peer_id: String,
    port: u16,
    uploaded: usize,
    downloaded: usize,
    left: usize,
    #[serde(serialize_with = "bool_to_u8")]
    compact: bool,
}

impl TrackerRequest {
    pub fn create_url(&self, tracker_url: &str) -> String {
        let params = serde_urlencoded::to_string(&self).unwrap();

        format!(
            "{}?{}&info_hash={}",
            tracker_url,
            params,
            Self::urlencode_bytes(&self.info_hash)
        )
    }

    fn urlencode_bytes(bytes: &[u8; 20]) -> String {
        let mut encoded = String::with_capacity(3 * bytes.len());
        for &b in bytes {
            encoded.push('%');
            encoded.push_str(&hex::encode(&[b]));
        }

        encoded
    }
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct TrackerResponse {
    interval: u32,
    #[serde(rename = "peers", with = "serde_bytes")]
    peers_bytes: Vec<u8>,
}

impl TrackerResponse {
    fn get_peers(&self) -> Vec<SocketAddrV4> {
        let mut peers = vec![];
        let n_peers = self.peers_bytes.len() / 6;
        for i in 0..n_peers {
            let p = &self.peers_bytes[i * 6..];
            peers.push(SocketAddrV4::new(
                Ipv4Addr::new(p[0], p[1], p[2], p[3]),
                u16::from_be_bytes([p[4], p[5]]),
            ));
        }

        peers
    }
}

struct Peer<'a> {
    addr: SocketAddrV4,
    file: &'a TorrentFile,
}

impl<'a> Peer<'a> {
    fn handshake(&self) -> PeerConnection {
        let mut stream = TcpStream::connect(self.addr).unwrap();

        stream.write(&[PROTOCOL_STRING.len() as u8]).unwrap();
        stream.write(PROTOCOL_STRING.as_bytes()).unwrap();
        stream.write(&[0; 8]).unwrap();
        stream.write(&self.file.info.hash()).unwrap();
        stream.write(PEER_ID.as_bytes()).unwrap();

        let mut n_pstring = [0u8];
        stream.read_exact(&mut n_pstring).unwrap();
        let mut pstring = vec![0u8; n_pstring[0] as usize];
        stream.read_exact(&mut pstring).unwrap();

        // eight reserved bytes, which are all set to zero (8 bytes)
        std::io::copy(&mut Read::by_ref(&mut stream).take(8), &mut std::io::sink()).unwrap();

        let mut info_hash = [0u8; 20];
        stream.read_exact(&mut info_hash).unwrap();

        let mut peer_id = [0u8; 20];
        stream.read_exact(&mut peer_id).unwrap();

        PeerConnection {
            protocol: pstring,
            info_hash,
            peer_id,
            stream,
            peer: self,
        }
    }
}

#[allow(dead_code)]
struct PeerConnection<'a> {
    protocol: Vec<u8>,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    stream: TcpStream,
    peer: &'a Peer<'a>,
}

impl<'a> PeerConnection<'a> {
    fn receive_message(&mut self) -> PeerMessage {
        let mut header = [0u8; 5];
        self.stream.read_exact(&mut header).unwrap();

        let length = Self::u32_from_bytes(&header);
        let id = header[4];

        let mut payload = vec![0u8; (length - 5) as usize];
        self.stream.read_exact(&mut payload).unwrap();

        match id {
            5 => PeerMessage::Bitfield,
            2 => PeerMessage::Interested,
            1 => PeerMessage::Unchoke,
            6 => PeerMessage::Request(RequestPayload {
                index: Self::u32_from_bytes(&payload),
                begin: Self::u32_from_bytes(&payload[4..]),
                length: Self::u32_from_bytes(&payload[8..]),
            }),
            7 => PeerMessage::Piece(PiecePayload {
                index: Self::u32_from_bytes(&payload),
                begin: Self::u32_from_bytes(&payload[4..]),
                block: {
                    payload.drain(0..8);
                    payload
                },
            }),
            _ => panic!("Unexpected message"),
        }
    }

    fn send_message(&mut self, message: PeerMessage) {
        match message {
            PeerMessage::Interested => {
                self.stream.write(&[2]).unwrap();
            }
            PeerMessage::Request(payload) => {
                self.stream.write(&[6]).unwrap();
                self.stream.write(&payload.index.to_be_bytes()).unwrap();
                self.stream.write(&payload.begin.to_be_bytes()).unwrap();
                self.stream.write(&payload.length.to_be_bytes()).unwrap();
            }
            _ => panic!("Unabled to send the message"),
        };
    }

    fn download_piece(&mut self, index: u32, output: &str) {
        let PeerMessage::Bitfield = self.receive_message() else {
            panic!("Didn't receive the bitfield message")
        };

        self.send_message(PeerMessage::Interested);

        let PeerMessage::Unchoke = self.receive_message() else {
            panic!("Didn't receive the unchoke message")
        };

        let plength = self.peer.file.info.plength as u32;
        let mut downloaded = 0;

        let mut piece_data = Vec::<u8>::with_capacity(plength as usize);

        while downloaded < plength {
            let length = u32::min(BLOCK_SIZE, plength - downloaded);
            self.send_message(PeerMessage::Request(RequestPayload {
                index,
                begin: downloaded,
                length,
            }));

            let PeerMessage::Piece(payload) = self.receive_message() else {
                panic!("Didn't receive the piece message")
            };

            let mut block_data = Cursor::new(payload.block);
            std::io::copy(&mut block_data, &mut piece_data).unwrap();

            downloaded += length;
        }

        let mut hasher = Sha1::new();
        hasher.update(&piece_data);
        let computed_hash = hasher.finalize();
        let piece_hash = self
            .peer
            .file
            .info
            .get_pieces()
            .nth(index as usize)
            .unwrap();

        if computed_hash.len() != piece_hash.len() {
            panic!("Hash mismatch");
        }

        for i in 0..computed_hash.len() {
            if computed_hash[i] != piece_hash[i] {
                panic!("Hash mismatch");
            }
        }

        fs::write(output, &piece_data).unwrap();
    }

    fn u32_from_bytes(data: &[u8]) -> u32 {
        u32::from_be_bytes([data[0], data[1], data[2], data[3]])
    }
}

enum PeerMessage {
    Bitfield,
    Interested,
    Unchoke,
    Request(RequestPayload),
    Piece(PiecePayload),
}

struct RequestPayload {
    index: u32,
    begin: u32,
    length: u32,
}

#[allow(dead_code)]
struct PiecePayload {
    index: u32,
    begin: u32,
    block: Vec<u8>,
}

fn bool_to_u8<S>(b: &bool, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_u8(if *b { 1 } else { 0 })
}

fn decode_bencoded_value(encoded_value: &str) -> Decode {
    // If encoded_value starts with a digit, it's a number
    let next = encoded_value.chars().next().unwrap();
    if next.is_digit(10) {
        // Example: "5:hello" -> "hello"
        let colon_index = encoded_value.find(':').unwrap();
        let number_string = &encoded_value[..colon_index];
        let number = number_string.parse::<i64>().unwrap();
        let string = &encoded_value[colon_index + 1..colon_index + 1 + number as usize];
        return Decode::new(
            colon_index + 1 + (number as usize),
            serde_json::Value::String(string.to_string()),
        );
    } else if next == 'i' {
        let e_index = encoded_value.find('e').unwrap();
        let digits = &encoded_value[1..e_index];
        return Decode::new(e_index + 1, digits.parse::<i64>().unwrap().into());
    } else if next == 'l' {
        let mut remaining = &encoded_value[1..];
        let mut items = vec![];
        let mut length = 1;
        loop {
            if remaining.chars().next().unwrap() == 'e' {
                length += 1;
                return Decode::new(length, items.into());
            }

            let next_item = decode_bencoded_value(remaining);
            remaining = &remaining[next_item.length..];
            length += next_item.length;
            items.push(next_item.value);
        }
    } else if next == 'd' {
        let mut remaining = &encoded_value[1..];
        let mut dict = Map::new();
        let mut length = 1;
        loop {
            if remaining.chars().next().unwrap() == 'e' {
                length += 1;
                return Decode::new(length, serde_json::Value::Object(dict));
            }

            let next_key = decode_bencoded_value(remaining);
            if let serde_json::Value::String(key_str) = next_key.value {
                length += next_key.length;
                remaining = &remaining[next_key.length..];
                let next_value = decode_bencoded_value(remaining);
                length += next_value.length;
                remaining = &remaining[next_value.length..];
                dict.insert(key_str, next_value.value);
            } else {
                panic!("Expected string key");
            }
        }
    } else {
        panic!("Unhandled encoded value: {}", encoded_value)
    }
}

// Usage: your_bittorrent.sh decode "<encoded_value>"
fn main() {
    let args: Vec<String> = env::args().collect();
    let command = &args[1];
    if command == "decode" {
        // You can use print statements as follows for debugging, they'll be visible when running tests.
        // println!("Logs from your program will appear here!");

        // Uncomment this block to pass the first stage
        let encoded_value = &args[2];
        let decoded_value = decode_bencoded_value(encoded_value);
        println!("{}", decoded_value.value.to_string());
    } else if command == "info" {
        let tfile = TorrentFile::from_file(&args[2]);
        println!("Tracker URL: {}", tfile.announce);
        println!("Length: {}", tfile.info.length);
        println!("Info Hash: {}", hex::encode(&tfile.info.hash()));

        println!("Piece Length: {}", tfile.info.plength);
        println!("Piece Hashes:");
        for p in tfile.info.get_pieces() {
            println!("{}", hex::encode(p));
        }
    } else if command == "peers" {
        let tfile = TorrentFile::from_file(&args[2]);
        let peers = tfile.find_peers();
        for peer in peers {
            println!("{}", peer.addr);
        }
    } else if command == "handshake" {
        let tfile = TorrentFile::from_file(&args[2]);
        let peer = tfile.create_peer(SocketAddrV4::from_str(&args[3]).unwrap());
        let handshake = peer.handshake();
        println!("Peer ID: {}", hex::encode(handshake.peer_id));
    } else if command == "download_piece" {
        let output = &args[3];
        let tfile = TorrentFile::from_file(&args[4]);
        let piece = u32::from_str(&args[5]).unwrap();
        let mut peers = tfile.find_peers();
        let peer = peers.next().unwrap();
        let mut connection = peer.handshake();
        connection.download_piece(piece, output);
    } else {
        println!("unknown command: {}", args[1])
    }
}
