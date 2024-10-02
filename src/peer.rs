use std::{
    io::{Cursor, Read, Write},
    net::{SocketAddrV4, TcpStream},
};

use sha1::{Digest, Sha1};

use crate::{consts::PEER_ID, tfile::TorrentFile};

const PROTOCOL_STRING: &str = "BitTorrent protocol";
const BLOCK_SIZE: u32 = 1 << 14;

pub struct Peer<'a> {
    pub addr: SocketAddrV4,
    file: &'a TorrentFile,
}

impl<'a> Peer<'a> {
    pub fn new(addr: SocketAddrV4, file: &'a TorrentFile) -> Self {
        Self { addr, file }
    }

    pub fn handshake(&self) -> PeerConnection {
        let mut stream = TcpStream::connect(self.addr).unwrap();

        stream.write(&[PROTOCOL_STRING.len() as u8]).unwrap();
        stream.write(PROTOCOL_STRING.as_bytes()).unwrap();
        stream.write(&[0; 8]).unwrap();
        stream.write(&self.file.info.hash()).unwrap();
        stream.write(PEER_ID.as_bytes()).unwrap();
        stream.flush().unwrap();

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
            initiated: false,
        }
    }
}

#[allow(dead_code)]
pub struct PeerConnection<'a> {
    protocol: Vec<u8>,
    info_hash: [u8; 20],
    pub peer_id: [u8; 20],
    stream: TcpStream,
    peer: &'a Peer<'a>,
    initiated: bool,
}

impl<'a> PeerConnection<'a> {
    fn receive_message(&mut self) -> PeerMessage {
        // println!("receiving");
        let mut header = [0u8; 4];
        self.stream.read_exact(&mut header).unwrap();

        let mut length = Self::u32_from_bytes(&header);

        // println!("received length: {0}", length);

        if length == 0 {
            // Messages of length zero are keepalives, and ignored. Keepalives are generally
            // sent once every two minutes, but note that timeouts can be done much more
            // quickly when data is expected.
            return self.receive_message();
        }

        length -= 1;

        // println!("received id: {0}", length);

        let mut id = [0u8];
        self.stream.read_exact(&mut id).unwrap();
        let id = id[0];

        // println!("received id: {0}", id);

        let mut payload = vec![0u8; length as usize];
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
            _ => self.receive_message(),
        }
    }

    fn send_message(&mut self, message: PeerMessage) {
        match message {
            PeerMessage::Interested => {
                // length
                self.stream.write(&1u32.to_be_bytes()).unwrap();
                // id
                self.stream.write(&[2u8]).unwrap();
            }
            PeerMessage::Request(payload) => {
                // length
                self.stream.write(&13u32.to_be_bytes()).unwrap();
                // id
                self.stream.write(&[6u8]).unwrap();
                self.stream.write(&payload.index.to_be_bytes()).unwrap();
                self.stream.write(&payload.begin.to_be_bytes()).unwrap();
                self.stream.write(&payload.length.to_be_bytes()).unwrap();
            }
            _ => panic!("Unabled to send the message"),
        };

        self.stream.flush().unwrap();
    }

    pub fn download_piece<W>(&mut self, index: u32, output: &mut W)
    where
        W: Write,
    {
        if !self.initiated {
            let PeerMessage::Bitfield = self.receive_message() else {
                panic!("Didn't receive the bitfield message")
            };

            self.send_message(PeerMessage::Interested);

            let PeerMessage::Unchoke = self.receive_message() else {
                panic!("Didn't receive the unchoke message")
            };

            self.initiated = true;
        }

        let plength = self.peer.file.info.nth_plength(index as usize) as u32;

        let mut begin = 0u32;

        let mut piece_data = Vec::<u8>::with_capacity(plength as usize);

        while begin < plength {
            let length = if begin + BLOCK_SIZE < plength {
                BLOCK_SIZE
            } else {
                plength - begin
            };

            // println!("Downloading: {}, {}, {}", index, begin, length);

            self.send_message(PeerMessage::Request(RequestPayload {
                index,
                begin,
                length,
            }));

            let PeerMessage::Piece(payload) = self.receive_message() else {
                panic!("Didn't receive the piece message")
            };

            let mut block_data = Cursor::new(payload.block);
            std::io::copy(&mut block_data, &mut piece_data).unwrap();

            begin += length;
        }

        if begin == 0 {
            // in case index >= number of the pieces
            return;
        }

        let mut hasher = Sha1::new();
        hasher.update(&piece_data);
        let computed_hash = hasher.finalize();
        let piece_hash = self.peer.file.info.pieces().nth(index as usize).unwrap();

        if computed_hash.len() != piece_hash.len() {
            panic!("Hash mismatch");
        }

        for i in 0..computed_hash.len() {
            if computed_hash[i] != piece_hash[i] {
                panic!("Hash mismatch");
            }
        }

        output.write_all(&piece_data).unwrap();
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
