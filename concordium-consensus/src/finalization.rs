use byteorder::{ByteOrder, NetworkEndian, WriteBytesExt};
use failure::Fallible;

use std::{
    fmt,
    io::{Cursor, Read, Write},
    mem::size_of,
};

use crate::{block::*, common::*};

const HEADER: u8 = SESSION_ID as u8
    + size_of::<FinalizationIndex>() as u8
    + size_of::<BlockHeight>() as u8
    + size_of::<Party>() as u8;
const SIGNATURE: u8 = 8 + 64; // FIXME: unnecessary 8B prefix
const WMVBA_TYPE: u8 = 1;
const VAL: u8 = BLOCK_HASH;
const TICKET: u8 = 80;

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct FinalizationMessage {
    header:    FinalizationMessageHeader,
    message:   WmvbaMessage,
    signature: ByteString,
}

impl fmt::Display for FinalizationMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "finalization message ({})", self.message)
    }
}

impl FinalizationMessage {
    pub fn deserialize(bytes: &[u8]) -> Fallible<Self> {
        // debug_deserialization!("FinalizationMessage", bytes);

        let mut cursor = Cursor::new(bytes);

        let header =
            FinalizationMessageHeader::deserialize(&read_const_sized!(&mut cursor, HEADER))?;
        let message_size = bytes.len() - cursor.position() as usize - SIGNATURE as usize;
        let message = WmvbaMessage::deserialize(&read_sized!(&mut cursor, message_size))?;
        let signature = ByteString::new(&read_const_sized!(&mut cursor, SIGNATURE));

        let msg = FinalizationMessage {
            header,
            message,
            signature,
        };

        check_serialization!(msg, cursor);

        Ok(msg)
    }

    pub fn serialize(&self) -> Box<[u8]> {
        [
            &self.header.serialize(),
            &self.message.serialize(),
            self.signature.as_ref(),
        ]
        .concat()
        .into_boxed_slice()
    }
}

pub type FinalizationIndex = u64;

type Party = u32;

#[derive(Debug, PartialEq, Eq, Hash)]
struct FinalizationMessageHeader {
    session_id: SessionId,
    index:      FinalizationIndex,
    delta:      BlockHeight,
    sender:     Party,
}

impl FinalizationMessageHeader {
    pub fn deserialize(bytes: &[u8]) -> Fallible<Self> {
        let mut cursor = Cursor::new(bytes);

        let session_id = SessionId::deserialize(&read_const_sized!(&mut cursor, SESSION_ID))?;
        let index = NetworkEndian::read_u64(&read_const_sized!(&mut cursor, 8));
        let delta = NetworkEndian::read_u64(&read_const_sized!(&mut cursor, 8));
        let sender = NetworkEndian::read_u32(&read_const_sized!(&mut cursor, 4));

        let header = FinalizationMessageHeader {
            session_id,
            index,
            delta,
            sender,
        };

        check_serialization!(header, cursor);

        Ok(header)
    }

    pub fn serialize(&self) -> Box<[u8]> {
        let mut cursor = create_serialization_cursor(
            SESSION_ID as usize
                + size_of::<FinalizationIndex>()
                + size_of::<BlockHeight>()
                + size_of::<Party>(),
        );

        let _ = cursor.write_all(&self.session_id.serialize());
        let _ = cursor.write_u64::<NetworkEndian>(self.index);
        let _ = cursor.write_u64::<NetworkEndian>(self.delta);
        let _ = cursor.write_u32::<NetworkEndian>(self.sender);

        cursor.into_inner()
    }
}

type Val = BlockHash;

#[derive(Debug, PartialEq, Eq, Hash)]
// FIXME: this one needs a better name
enum WmvbaMessage {
    Proposal(Val),
    Vote(Option<Val>),
    AbbaInput(AbbaInput),
    CssSeen(CssSeen),
    CssDoneReporting(CssDoneReporting),
    AreWeDone(bool),
    WitnessCreator(Val),
}

impl fmt::Display for WmvbaMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let output = match self {
            WmvbaMessage::Proposal(prop) => format!("Proposal: {:?}", prop),
            WmvbaMessage::Vote(vote) => format!(
                "Vote: {}",
                if let Some(v) = vote {
                    format!("{:?}", v)
                } else {
                    "blank".to_owned()
                }
            ),
            WmvbaMessage::AbbaInput(abba) => format!("AbbaInput phase {}", abba.phase),
            WmvbaMessage::CssSeen(css) => format!("CssSeen phase {}", css.phase),
            WmvbaMessage::CssDoneReporting(c) => format!("CssDoneReporting phase {}", c.phase),
            WmvbaMessage::AreWeDone(arewe) => if *arewe {
                "We're done"
            } else {
                "We're not done"
            }
            .to_owned(),
            WmvbaMessage::WitnessCreator(_) => "WitnessCreator".to_owned(),
        };

        write!(f, "{}", output)
    }
}

impl WmvbaMessage {
    pub fn deserialize(bytes: &[u8]) -> Fallible<Self> {
        let mut cursor = Cursor::new(bytes);

        let message_type = &read_const_sized!(&mut cursor, WMVBA_TYPE)[0];

        let msg = match message_type {
            0 => WmvbaMessage::Proposal(HashBytes::new(&read_const_sized!(&mut cursor, VAL))),
            1 => WmvbaMessage::Vote(None),
            2 => WmvbaMessage::Vote(Some(HashBytes::new(&read_const_sized!(&mut cursor, VAL)))),
            3 => WmvbaMessage::AbbaInput(AbbaInput::deserialize(&read_all(&mut cursor)?, false)?),
            4 => WmvbaMessage::AbbaInput(AbbaInput::deserialize(&read_all(&mut cursor)?, true)?),
            5 => WmvbaMessage::CssSeen(CssSeen::deserialize(&read_all(&mut cursor)?, false)?),
            6 => WmvbaMessage::CssSeen(CssSeen::deserialize(&read_all(&mut cursor)?, true)?),
            7 => WmvbaMessage::CssDoneReporting(CssDoneReporting::deserialize(&read_all(
                &mut cursor,
            )?)?),
            8 => WmvbaMessage::AreWeDone(false),
            9 => WmvbaMessage::AreWeDone(true),
            10 => {
                WmvbaMessage::WitnessCreator(HashBytes::new(&read_const_sized!(&mut cursor, VAL)))
            }
            n => panic!(
                "Deserialization of WMVBA message type No {} is not implemented!",
                n
            ),
        };

        check_serialization!(msg, cursor);

        Ok(msg)
    }

    pub fn serialize(&self) -> Box<[u8]> {
        let vec = match self {
            WmvbaMessage::Proposal(ref val) => [&[0], val.as_ref()].concat(),
            WmvbaMessage::Vote(vote) => match vote {
                None => vec![1],
                Some(val) => [&[2], val.as_ref()].concat(),
            },
            WmvbaMessage::AbbaInput(abba) => {
                if !abba.justified {
                    [&[3], &*abba.serialize()].concat()
                } else {
                    [&[4], &*abba.serialize()].concat()
                }
            }
            WmvbaMessage::CssSeen(css) => {
                if !css.saw {
                    [&[5], &*css.serialize()].concat()
                } else {
                    [&[6], &*css.serialize()].concat()
                }
            }
            WmvbaMessage::CssDoneReporting(cdr) => [&[7], &*cdr.serialize()].concat(),
            WmvbaMessage::AreWeDone(arewe) => match arewe {
                false => vec![8],
                true => vec![9],
            },
            WmvbaMessage::WitnessCreator(val) => [&[10], val.as_ref()].concat(),
        };

        vec.into_boxed_slice()
    }
}

type Phase = u32;

#[derive(Debug, PartialEq, Eq, Hash)]
struct AbbaInput {
    phase:     Phase,
    ticket:    Encoded,
    justified: bool, // FIXME: verify that this is what True/False means here
}

impl AbbaInput {
    pub fn deserialize(bytes: &[u8], justified: bool) -> Fallible<Self> {
        let mut cursor = Cursor::new(bytes);

        let phase = NetworkEndian::read_u32(&read_const_sized!(&mut cursor, 4));
        let ticket = Encoded::new(&read_const_sized!(&mut cursor, TICKET));

        let abba = AbbaInput {
            phase,
            ticket,
            justified,
        };

        check_serialization!(abba, cursor);

        Ok(abba)
    }

    pub fn serialize(&self) -> Box<[u8]> {
        let mut cursor = create_serialization_cursor(size_of::<Phase>() + TICKET as usize);

        let _ = cursor.write_u32::<NetworkEndian>(self.phase);
        let _ = cursor.write_all(&self.ticket);

        cursor.into_inner()
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct CssSeen {
    phase: Phase,
    party: Party,
    saw:   bool,
}

impl CssSeen {
    pub fn deserialize(bytes: &[u8], saw: bool) -> Fallible<Self> {
        let mut cursor = Cursor::new(bytes);

        let phase = NetworkEndian::read_u32(&read_const_sized!(&mut cursor, 4));
        let party = NetworkEndian::read_u32(&read_const_sized!(&mut cursor, 4));

        let css = CssSeen { phase, party, saw };

        check_serialization!(css, cursor);

        Ok(css)
    }

    pub fn serialize(&self) -> Box<[u8]> {
        let mut cursor = create_serialization_cursor(size_of::<Phase>() + size_of::<Party>());

        let _ = cursor.write_u32::<NetworkEndian>(self.phase);
        let _ = cursor.write_u32::<NetworkEndian>(self.party);

        cursor.into_inner()
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct CssDoneReporting {
    phase:       Phase,
    chose_false: Vec<Party>,
    chose_true:  Vec<Party>,
}

impl CssDoneReporting {
    pub fn deserialize(bytes: &[u8]) -> Fallible<Self> {
        let mut cursor = Cursor::new(bytes);

        let phase = NetworkEndian::read_u32(&read_const_sized!(&mut cursor, 4));

        let n_false = NetworkEndian::read_u64(&read_const_sized!(&mut cursor, 8));
        let mut chose_false = Vec::with_capacity(n_false as usize);
        for _ in 0..n_false {
            let party = NetworkEndian::read_u32(&read_const_sized!(&mut cursor, 4));
            chose_false.push(party);
        }

        let n_true = NetworkEndian::read_u64(&read_const_sized!(&mut cursor, 8));
        let mut chose_true = Vec::with_capacity(n_true as usize);
        for _ in 0..n_true {
            let party = NetworkEndian::read_u32(&read_const_sized!(&mut cursor, 4));
            chose_true.push(party);
        }

        let cssr = CssDoneReporting {
            phase,
            chose_false,
            chose_true,
        };

        check_serialization!(cssr, cursor);

        Ok(cssr)
    }

    pub fn serialize(&self) -> Box<[u8]> {
        let mut cursor = create_serialization_cursor(
            size_of::<Phase>()
                + 8 // u64 count of those who chose "false"
                + size_of::<Party>() * self.chose_false.len()
                + 8 // u64 count of those who chose "true"
                + size_of::<Party>() * self.chose_true.len(),
        );

        let _ = cursor.write_u32::<NetworkEndian>(self.phase);

        let _ = cursor.write_u64::<NetworkEndian>(self.chose_false.len() as u64);
        for party in &self.chose_false {
            let _ = cursor.write_u32::<NetworkEndian>(*party);
        }

        let _ = cursor.write_u64::<NetworkEndian>(self.chose_true.len() as u64);
        for party in &self.chose_true {
            let _ = cursor.write_u32::<NetworkEndian>(*party);
        }

        cursor.into_inner()
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct FinalizationRecord {
    index:         FinalizationIndex,
    block_pointer: BlockHash,
    proof:         FinalizationProof,
    delay:         BlockHeight,
}

impl fmt::Display for FinalizationRecord {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "finalization record ({:?})", self.block_pointer)
    }
}

impl FinalizationRecord {
    pub fn deserialize(bytes: &[u8]) -> Fallible<Self> {
        // debug_deserialization!("FinalizationRecord", bytes);

        let mut cursor = Cursor::new(bytes);

        let index = NetworkEndian::read_u64(&read_const_sized!(&mut cursor, 8));
        let block_pointer = HashBytes::new(&read_const_sized!(&mut cursor, BLOCK_HASH));
        let proof_size = bytes.len() - cursor.position() as usize - size_of::<BlockHeight>();
        let proof = FinalizationProof::deserialize(&read_sized!(&mut cursor, proof_size))?;
        let delay = NetworkEndian::read_u64(&read_const_sized!(&mut cursor, 8));

        let rec = FinalizationRecord {
            index,
            block_pointer,
            proof,
            delay,
        };

        check_serialization!(rec, cursor);

        Ok(rec)
    }

    pub fn serialize(&self) -> Box<[u8]> {
        let proof = self.proof.serialize();

        let mut cursor = create_serialization_cursor(
            size_of::<FinalizationIndex>()
                + self.block_pointer.len()
                + proof.len()
                + size_of::<BlockHeight>(),
        );

        let _ = cursor.write_u64::<NetworkEndian>(self.index);
        let _ = cursor.write_all(&self.block_pointer);
        let _ = cursor.write_all(&proof);
        let _ = cursor.write_u64::<NetworkEndian>(self.delay);

        cursor.into_inner()
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct FinalizationProof(Vec<(u32, Encoded)>);

impl FinalizationProof {
    pub fn deserialize(bytes: &[u8]) -> Fallible<Self> {
        let mut cursor = Cursor::new(bytes);

        let signature_count = NetworkEndian::read_u64(&read_const_sized!(&mut cursor, 8));

        let mut signatures = Vec::with_capacity(signature_count as usize);

        for _ in 0..signature_count {
            // FIXME: determine the use and apply a more informative name
            let tbd = NetworkEndian::read_u32(&read_const_sized!(&mut cursor, 4));
            let signature = Encoded::new(&read_const_sized!(&mut cursor, SIGNATURE));

            signatures.push((tbd, signature));
        }

        let proof = FinalizationProof(signatures);

        check_serialization!(proof, cursor);

        Ok(proof)
    }

    pub fn serialize(&self) -> Box<[u8]> {
        let mut cursor = create_serialization_cursor(8 + self.0.len() * (4 + SIGNATURE as usize));

        let _ = cursor.write_u64::<NetworkEndian>(self.0.len() as u64);

        // FIXME: determine the use and apply a more informative name
        for (tbd, signature) in &self.0 {
            let _ = cursor.write_u32::<NetworkEndian>(*tbd);
            let _ = cursor.write_all(signature);
        }

        cursor.into_inner()
    }
}
