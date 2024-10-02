use core::str;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{self, Map};
use sha1::{Digest, Sha1};
use std::{
    env, fs,
    net::{Ipv4Addr, SocketAddrV4},
};

const PEER_ID: &str = "00112233445566778899";

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

    fn get_hash(&self) -> [u8; 20] {
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

fn bool_to_u8<S>(b: &bool, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_u8(if *b { 1 } else { 0 })
}

#[allow(dead_code)]
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
        println!("Info Hash: {}", hex::encode(&tfile.info.get_hash()));

        println!("Piece Length: {}", tfile.info.plength);
        println!("Piece Hashes:");
        for p in tfile.info.get_pieces() {
            println!("{}", hex::encode(p));
        }
    } else if command == "peers" {
        let tfile = TorrentFile::from_file(&args[2]);
        let req = TrackerRequest {
            info_hash: tfile.info.get_hash(),
            peer_id: PEER_ID.into(),
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: tfile.info.length,
            compact: true,
        };

        let url = req.create_url(&tfile.announce);

        let client = reqwest::blocking::Client::new();
        let http_response = client.get(url).send().unwrap();

        let resp: TrackerResponse =
            serde_bencode::from_bytes(&http_response.bytes().unwrap()).unwrap();

        for peer in resp.get_peers() {
            println!("{}", peer);
        }
    } else {
        println!("unknown command: {}", args[1])
    }
}
