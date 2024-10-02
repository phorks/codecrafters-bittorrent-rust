use std::{
    fs,
    net::{Ipv4Addr, SocketAddrV4},
};

use serde::{Deserialize, Serialize, Serializer};
use sha1::{Digest, Sha1};

use crate::{consts::PEER_ID, peer::Peer};

#[derive(Serialize, Deserialize)]
pub struct TorrentFile {
    pub announce: String,
    pub info: TorrentFileInfo,
}

impl TorrentFile {
    pub fn from_file(path: &str) -> Self {
        let bytes = fs::read(path).unwrap();
        return serde_bencode::from_bytes(&bytes).unwrap();
    }

    pub fn create_peer(&self, addr: SocketAddrV4) -> Peer {
        Peer::new(addr, self)
    }

    pub fn find_peers(&self) -> impl Iterator<Item = Peer> {
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
            .map(move |addr| Peer::new(addr, &self))
    }
}

#[derive(Serialize, Deserialize)]
pub struct TorrentFileInfo {
    pub length: usize,
    pub name: String,
    #[serde(rename = "piece length")]
    pub plength: usize,
    #[serde(rename = "pieces", with = "serde_bytes")]
    pieces_data: Vec<u8>,
}

impl TorrentFileInfo {
    pub fn pieces(&self) -> impl Iterator<Item = &[u8]> {
        return self.pieces_data.chunks(20);
    }

    pub fn hash(&self) -> [u8; 20] {
        let data = &serde_bencode::to_bytes(&self).unwrap();
        let mut hasher = Sha1::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    pub fn n_pieces(&self) -> usize {
        self.pieces_data.len() / 20
    }

    pub fn nth_plength(&self, n: usize) -> usize {
        let n_pieces = self.n_pieces();
        if n < n_pieces - 1 {
            self.plength
        } else if n == (n_pieces - 1) {
            self.length % self.plength
        } else {
            0
        }
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

fn bool_to_u8<S>(b: &bool, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_u8(if *b { 1 } else { 0 })
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
