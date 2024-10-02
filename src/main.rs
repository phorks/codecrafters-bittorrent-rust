use core::str;
use serde_json::{self, Map};
use std::{env, fs, net::SocketAddrV4, str::FromStr};
use tfile::TorrentFile;
mod consts;
mod peer;
mod tfile;

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
        for p in tfile.info.pieces() {
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
        return;
        let output = &args[3];
        let tfile = TorrentFile::from_file(&args[4]);
        let piece = u32::from_str(&args[5]).unwrap();
        let mut peers = tfile.find_peers();
        let peer = peers.next().unwrap();
        let mut connection = peer.handshake();
        let mut output = fs::File::create(output).unwrap();
        connection.download_piece(piece, &mut output);
    } else if command == "download" {
        let output = &args[3];
        let tfile = TorrentFile::from_file(&args[4]);
        let peer = tfile.find_peers().next().unwrap();
        let mut output = fs::File::create(output).unwrap();
        let mut connection = peer.handshake();
        for piece in 0..tfile.info.n_pieces() as u32 {
            // println!("Downloading {}th piece", piece);
            connection.download_piece(piece, &mut output);
        }
    } else {
        println!("unknown command: {}", args[1])
    }
}
