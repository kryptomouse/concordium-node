use num_bigint::{BigUint, ToBigUint};
use num_traits::pow;
use rand::{rngs::OsRng, seq::SliceRandom};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::RwLock,
};

use crate::common::{ConnectionType, P2PNodeId, P2PPeer};

const KEY_SIZE: u16 = 256;
const BUCKET_SIZE: u8 = 20;

pub struct Bucket {
    pub peer:     P2PPeer,
    pub networks: HashSet<u16>,
}

pub struct Buckets {
    buckets: HashMap<u16, Vec<Bucket>>,
    // buckets: HashMap<u16, HashMap<P2PPeer, HashSet<u16>>>,
}

fn make_distance_table() -> [Range; KEY_SIZE] {
    let mut dist_table: [Range; KEY_SIZE] = [(0..1); KEY_SIZE];
    for i in 0..(KEY_SIZE as usize) {
        dist_table[i] = Range {
            start: pow(2_i8.to_biguint().unwrap(), i),
            end:   pow(2_i8.to_biguint().unwrap(), i + 1),
        }
    }
    dist_table
}

lazy_static! {
    static ref RNG: RwLock<OsRng> = { RwLock::new(OsRng::new().unwrap()) };
    static ref DISTANCE_TABLE: [Range; KEY_SIZE] = make_distance_table();
}

impl Buckets {
    pub fn new() -> Buckets {
        let mut buckets = HashMap::with_capacity(KEY_SIZE);
        for i in 0..KEY_SIZE {
            buckets.insert(i, HashMap::new())
        }

        Buckets { buckets }
    }

    pub fn distance(&self, from: &P2PNodeId, to: &P2PNodeId) -> BigUint {
        from.get_id() ^ to.get_id()
    }

    pub fn insert_into_bucket(&mut self, node: &P2PPeer, own_id: &P2PNodeId, nids: HashSet<u16>) {
        let dist = self.distance(&own_id, &node.id());
        for i in 0..KEY_SIZE {
            if let Some(bucket_list) = self.buckets.get_mut(&i) {
                bucket_list.retain(|ref ele| ele.peer != *node);

                if let Ok(index) = DISTANCE_TABLE.binary_search_by(dist) {}
                if dist >= pow(2_i8.to_biguint().unwrap(), i as usize)
                    && dist < pow(2_i8.to_biguint().unwrap(), (i as usize) + 1)
                {
                    if bucket_list.len() >= BUCKET_SIZE as usize {
                        bucket_list.remove(0);
                    }
                    bucket_list.push(Bucket {
                        peer:     node.clone(),
                        networks: nids,
                    });
                    break;
                }
            }
        }
    }

    pub fn update_network_ids(&mut self, node: &P2PPeer, nids: Vec<u16>) {
        for i in 0..KEY_SIZE {
            match self.buckets.get_mut(&i) {
                Some(x) => {
                    x.retain(|ref ele| ele.0 != *node);
                    x.push((node.clone(), nids.clone()));
                    break;
                }
                None => {
                    error!("Couldn't get buck as mutable");
                }
            }
        }
    }

    fn _find_bucket_id(&mut self, own_id: P2PNodeId, id: P2PNodeId) -> Option<u16> {
        let dist = self.distance(&own_id, &id);
        let mut ret: i32 = -1;
        for i in 0..KEY_SIZE {
            if dist >= pow(2_i8.to_biguint().unwrap(), i as usize)
                && dist < pow(2_i8.to_biguint().unwrap(), (i as usize) + 1)
            {
                ret = i as i32;
            }
        }

        if ret == -1 {
            None
        } else {
            Some(ret as u16)
        }
    }

    pub fn closest_nodes(&self, _id: &P2PNodeId) -> Vec<P2PPeer> {
        let mut ret: Vec<P2PPeer> = Vec::with_capacity(KEY_SIZE as usize);
        let mut count = 0;
        for (_, bucket) in &self.buckets {
            // Fix later to do correctly
            if count < KEY_SIZE {
                for peer in bucket {
                    if count < KEY_SIZE {
                        ret.push(peer.0.clone());
                        count += 1;
                    } else {
                        break;
                    }
                }
            } else {
                break;
            }
        }
        ret
    }

    pub fn clean_peers(&mut self, retain_minimum: usize) {
        let self_len = self.len();
        for i in 0..KEY_SIZE {
            match self.buckets.get_mut(&i) {
                Some(x) => {
                    if retain_minimum < x.len() {
                        debug!("Cleaning buckets currently at {}", self_len);
                        x.sort_by(|a, b| {
                            use std::cmp::Ordering;
                            if a > b {
                                return Ordering::Less;
                            } else if a < b {
                                return Ordering::Greater;
                            } else {
                                return Ordering::Equal;
                            }
                        });
                        x.drain(retain_minimum..);
                    }
                }
                None => {
                    error!("Couldn't get bucket as mutable");
                }
            }
        }
    }

    pub fn get_all_nodes(&self, sender: Option<&P2PPeer>, networks: &[u16]) -> Vec<P2PPeer> {
        let mut ret: Vec<P2PPeer> = Vec::new();
        match sender {
            Some(sender_peer) => {
                for (_, bucket) in &self.buckets {
                    for peer in bucket {
                        if sender_peer != &peer.0
                            && peer.0.connection_type() == ConnectionType::Node
                            && (networks.len() == 0 || peer.1.iter().any(|x| networks.contains(x)))
                        {
                            ret.push(peer.0.clone());
                        }
                    }
                }
            }
            None => {
                for (_, bucket) in &self.buckets {
                    for peer in bucket {
                        if peer.0.connection_type() == ConnectionType::Node
                            && (networks.len() == 0 || peer.1.iter().any(|x| networks.contains(x)))
                        {
                            ret.push(peer.0.clone());
                        }
                    }
                }
            }
        }

        ret
    }

    pub fn len(&self) -> usize { self.buckets.iter().map(|(_, y)| y.len()).sum() }

    pub fn get_random_nodes(&self, sender: &P2PPeer, amount: usize, nids: &[u16]) -> Vec<P2PPeer> {
        match safe_write!(RNG) {
            Ok(ref mut rng) => self
                .get_all_nodes(Some(sender), nids)
                .choose_multiple(&mut **rng, amount)
                .cloned()
                .collect(),
            _ => vec![],
        }
    }
}
