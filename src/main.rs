use bytes::Bytes;
use hex::encode;
use serde::Deserialize;
use serde_json::{self, Map};
use sha1::{Digest, Sha1};
use std::{char, collections::HashMap, env, fs, ops::Index};

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

#[derive(Deserialize)]
struct TorrentFile {
    announce: String,
    info: TorrentFileInfo,
}

#[derive(Deserialize)]
struct TorrentFileInfo {
    length: usize,
    name: String,
    #[serde(rename = "piece length")]
    n_pieces: usize,
    #[serde(with = "serde_bytes")]
    pieces: Vec<u8>,
}

#[allow(dead_code)]
fn decode_bencoded_value(encoded_value: &str) -> Decode {
    // If encoded_value starts with a digit, it's a number
    let x: &[u8];

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
        let file_name = &args[2];
        let bytes = fs::read(file_name).unwrap();
        let torrent_file: TorrentFile = serde_bencode::from_bytes(&bytes).unwrap();
        println!("Tracker URL: {}", torrent_file.announce);
        println!("Length: {}", torrent_file.info.length);

        let mut hasher = Sha1::new();
        hasher.update(torrent_file.info.pieces);
        let hash = hasher.finalize();
        println!("Info Hash: {}", hex::encode(hash));
    } else {
        println!("unknown command: {}", args[1])
    }
}
