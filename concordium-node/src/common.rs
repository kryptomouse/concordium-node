use utils;
use num_bigint::BigUint;
use num_traits::Num;
use std::net::IpAddr;
use std::str::FromStr;
use std::cmp::Ordering;

const PROTOCOL_NAME: &'static str = "CONCORDIUMP2P";
const PROTOCOL_VERSION: &'static str = "001";
const PROTOCOL_NODE_ID_LENGTH:usize = 64;

const PROTOCOL_MESSAGE_TYPE_REQUEST_PING: &'static str = "0001";
const PROTOCOL_MESSAGE_TYPE_REQUEST_FINDNODE: &'static str = "0002";
const PROTOCOL_MESSAGE_TYPE_REQUEST_BANNODE: &'static str = "0003";
const PROTOCOL_MESSAGE_TYPE_RESPONSE_PONG: &'static str = "1001";
const PROTOCOL_MESSAGE_TYPE_RESPONSE_FINDNODE: &'static str = "1002";
const PROTOCOL_MESSAGE_TYPE_RESPONSE_BANNODE: &'static str = "1003";
const PROTOCOL_MESSAGE_TYPE_DIRECT_MESSAGE: &'static str = "2001";
const PROTOCOL_MESSAGE_TYPE_BROADCASTED_MESSAGE: &'static str = "2002";

#[derive(Debug,Clone)]
pub enum NetworkMessage {
    NetworkRequest(NetworkRequest),
    NetworkResponse(NetworkResponse),
    NetworkPacket(NetworkPacket),
    UnknownMessage,
    InvalidMessage
}

impl NetworkMessage {
    pub fn deserialize(bytes: &str) -> NetworkMessage {
        let protocol_name_length = PROTOCOL_NAME.len();
        let protocol_version_length = PROTOCOL_VERSION.len();
        if bytes.len() >= protocol_name_length && bytes[..protocol_name_length] == *PROTOCOL_NAME  {
            if bytes.len() >= protocol_name_length+protocol_version_length && bytes[protocol_name_length..(protocol_name_length+protocol_version_length)] == *PROTOCOL_VERSION  {
                if bytes.len() < protocol_name_length+protocol_version_length+4 {
                    return NetworkMessage::InvalidMessage;
                }
                let message_type_id = &bytes[(protocol_name_length+protocol_version_length)..(protocol_name_length+protocol_version_length+4)];
                let inner_msg_size = protocol_name_length+protocol_version_length+4;
                match message_type_id as &str{
                    PROTOCOL_MESSAGE_TYPE_REQUEST_PING => {
                        let sender = P2PPeer::deserialize(&bytes[inner_msg_size..]);
                        match sender {
                            Some(peer) => NetworkMessage::NetworkRequest(NetworkRequest::Ping(peer)),
                            _ => NetworkMessage::InvalidMessage
                        }
                    },
                    PROTOCOL_MESSAGE_TYPE_RESPONSE_PONG => {
                         let sender = P2PPeer::deserialize(&bytes[inner_msg_size..]);
                        match sender {
                            Some(peer) => NetworkMessage::NetworkResponse(NetworkResponse::Pong(peer)),
                            _ => NetworkMessage::InvalidMessage
                        }
                    },
                    PROTOCOL_MESSAGE_TYPE_REQUEST_FINDNODE => {
                        if bytes.len() < protocol_name_length+protocol_version_length+4+PROTOCOL_NODE_ID_LENGTH+3 {
                            return NetworkMessage::InvalidMessage;
                        }
                        let sender = P2PPeer::deserialize(&bytes[inner_msg_size..]);
                        match sender {
                            Some(sender) => {
                                let sender_len = sender.serialize().len();
                                if bytes.len() != sender_len+protocol_name_length+protocol_version_length+4+PROTOCOL_NODE_ID_LENGTH {
                                    return NetworkMessage::InvalidMessage;
                                }
                                let node_id = &bytes[(inner_msg_size+sender_len)..];

                                NetworkMessage::NetworkRequest(NetworkRequest::FindNode(sender, P2PNodeId::from_string(node_id.to_string())))
                            },
                            _ => NetworkMessage::InvalidMessage
                        }
                        
                    },
                    PROTOCOL_MESSAGE_TYPE_REQUEST_BANNODE => {
                        if bytes.len() < protocol_name_length+protocol_version_length+4+PROTOCOL_NODE_ID_LENGTH+3 {
                            return NetworkMessage::InvalidMessage;
                        }
                        let sender = P2PPeer::deserialize(&bytes[inner_msg_size..]);
                        match sender {
                            Some(sender) => {
                                let sender_len = sender.serialize().len();
                                if bytes.len() != sender_len+protocol_name_length+protocol_version_length+4+PROTOCOL_NODE_ID_LENGTH {
                                    return NetworkMessage::InvalidMessage;
                                }
                                let node_id = &bytes[(inner_msg_size+sender_len)..];

                                NetworkMessage::NetworkRequest(NetworkRequest::BanNode(sender, P2PNodeId::from_string(node_id.to_string())))
                            },
                            _ => NetworkMessage::InvalidMessage
                        }
                        
                    },
                    PROTOCOL_MESSAGE_TYPE_RESPONSE_FINDNODE => {
                        let inner_msg: &str = &bytes[inner_msg_size..];
                        let sender = P2PPeer::deserialize(&bytes[inner_msg_size..]);
                        match sender {
                            Some(sender) => {
                                let sender_len = sender.serialize().len();
                                if inner_msg.len() < (3+sender_len) {
                                    return NetworkMessage::InvalidMessage;
                                }
                                let peers_count:Option<usize> = match &inner_msg[sender_len..(3+sender_len)].parse::<usize>() {
                                    Ok(n) => Some(*n),
                                    Err(_) => None
                                };
                                if peers_count.is_none() || peers_count.unwrap() == 0 {
                                    return NetworkMessage::NetworkResponse(NetworkResponse::FindNode(sender, vec![]));
                                }

                                let mut current_peer_start:usize = 3+sender_len;
                                let mut peers:Vec<P2PPeer> = vec![];
                                for _ in 0..peers_count.unwrap() {
                                    match P2PPeer::deserialize(&inner_msg[current_peer_start..]) {
                                        Some(peer) => {
                                            current_peer_start += &peer.serialize().len();
                                            peers.push(peer);
                                        }
                                        _ => return NetworkMessage::InvalidMessage
                                    }
                                }
                                return NetworkMessage::NetworkResponse(NetworkResponse::FindNode(sender,peers));
                            },
                            _ => NetworkMessage::InvalidMessage
                        }
                    },
                    PROTOCOL_MESSAGE_TYPE_DIRECT_MESSAGE => {
                        if &bytes.len() < &((inner_msg_size+10)) {
                            return NetworkMessage::InvalidMessage
                        }
                        let sender = P2PPeer::deserialize(&bytes[inner_msg_size..]);
                        match sender {
                            Some(peer) => {
                                let sender_len = &peer.serialize().len();
                                 if bytes[(inner_msg_size+sender_len)..].len() < 10 {
                                    return NetworkMessage::InvalidMessage
                                }
                                match bytes[(inner_msg_size+sender_len)..(inner_msg_size+10+sender_len)].parse::<usize>() {
                                    Ok(csize) => {
                                         if bytes[(inner_msg_size+sender_len+10)..].len() != csize {
                                            return NetworkMessage::InvalidMessage
                                        }
                                        return NetworkMessage::NetworkPacket(NetworkPacket::DirectMessage(peer, bytes[(inner_msg_size+10+sender_len)..(inner_msg_size+10+csize+sender_len)].to_string()))
                                    },
                                    Err(_) => {
                                        return NetworkMessage::InvalidMessage
                                    }
                                }
                            },
                            _ => NetworkMessage::InvalidMessage
                        }
                    },
                    PROTOCOL_MESSAGE_TYPE_BROADCASTED_MESSAGE => {
                        if &bytes.len() < &((inner_msg_size+10)) {
                            return NetworkMessage::InvalidMessage
                        }
                        let sender = P2PPeer::deserialize(&bytes[inner_msg_size..]);
                        match sender {
                            Some(peer) => {
                                let sender_len = &peer.serialize().len();
                                if bytes[(inner_msg_size+sender_len)..].len() < 10 {
                                    return NetworkMessage::InvalidMessage
                                }
                                match bytes[(inner_msg_size+sender_len)..(inner_msg_size+sender_len+10)].parse::<usize>() {
                                    Ok(csize) => {
                                        if bytes[(inner_msg_size+sender_len+10)..].len() != csize {
                                            return NetworkMessage::InvalidMessage
                                        }
                                        return NetworkMessage::NetworkPacket(NetworkPacket::BroadcastedMessage(peer, bytes[(sender_len+inner_msg_size+10)..(inner_msg_size+10+csize+sender_len)].to_string()))
                                    },
                                    Err(_) => {
                                        return NetworkMessage::InvalidMessage
                                    }
                                }
                            },
                            _ => NetworkMessage::InvalidMessage
                        }
                    }
                    _ => NetworkMessage::UnknownMessage
                }
            } else { 
                NetworkMessage::InvalidMessage
            }
        } else {
            NetworkMessage::InvalidMessage
        }
    }
}

#[derive(Debug,Clone)]
pub enum NetworkPacket {
    DirectMessage(P2PPeer, String),
    BroadcastedMessage(P2PPeer, String)
}

impl NetworkPacket {
    pub fn serialize(&self) -> String {
        match self {
            NetworkPacket::DirectMessage(me,msg) => format!("{}{}{}{}{:010}{}", PROTOCOL_NAME, PROTOCOL_VERSION, PROTOCOL_MESSAGE_TYPE_DIRECT_MESSAGE,me.serialize(),msg.len(),msg ), 
            NetworkPacket::BroadcastedMessage(me,msg) => format!("{}{}{}{}{:010}{}", PROTOCOL_NAME, PROTOCOL_VERSION, PROTOCOL_MESSAGE_TYPE_BROADCASTED_MESSAGE,me.serialize(),msg.len(),msg  )
        }
    }
}

#[derive(Debug,Clone)]
pub enum NetworkRequest {
    Ping(P2PPeer),
    FindNode(P2PPeer,P2PNodeId),
    BanNode(P2PPeer, P2PNodeId),
}

impl NetworkRequest {
     pub fn serialize(&self) -> String {
        match self {
            NetworkRequest::Ping(me) => format!("{}{}{}{}", PROTOCOL_NAME, PROTOCOL_VERSION, PROTOCOL_MESSAGE_TYPE_REQUEST_PING, me.serialize()),
            NetworkRequest::FindNode(me, id) => format!("{}{}{}{}{:x}", PROTOCOL_NAME, PROTOCOL_VERSION, PROTOCOL_MESSAGE_TYPE_REQUEST_FINDNODE,me.serialize(), id.get_id()),
            NetworkRequest::BanNode(me, id) => format!("{}{}{}{}{:x}", PROTOCOL_NAME, PROTOCOL_VERSION, PROTOCOL_MESSAGE_TYPE_REQUEST_BANNODE,me.serialize(), id.get_id() )
        }
    }
}

#[derive(Debug,Clone)]
pub enum NetworkResponse {
    Pong(P2PPeer),
    FindNode(P2PPeer, Vec<P2PPeer>),
    BanNode(P2PPeer, bool),
}

impl NetworkResponse {
     pub fn serialize(&self) -> String {
        match self {
            NetworkResponse::Pong(me) => format!("{}{}{}{}", PROTOCOL_NAME, PROTOCOL_VERSION, PROTOCOL_MESSAGE_TYPE_RESPONSE_PONG, me.serialize()),
            NetworkResponse::FindNode(me, peers) => {
                let mut buffer = String::new();
                for peer in peers.iter() {
                    buffer.push_str(&peer.serialize()[..]);
                }
                format!("{}{}{}{}{:03}{}", PROTOCOL_NAME, PROTOCOL_VERSION, PROTOCOL_MESSAGE_TYPE_RESPONSE_FINDNODE, me.serialize(), peers.len(), buffer)
            },
            NetworkResponse::BanNode(me, ok) => {
                format!("{}{}{}{}{}", PROTOCOL_NAME, PROTOCOL_VERSION, PROTOCOL_MESSAGE_TYPE_RESPONSE_BANNODE, me.serialize(), ok)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct P2PPeer {
    ip: IpAddr,
    port: u16,
    id: P2PNodeId,
}

impl P2PPeer {
    pub fn new(ip: IpAddr, port: u16) -> Self {
        P2PPeer {
            ip,
            port,
            id: P2PNodeId::from_ip_port(ip, port),
        }
    }

    pub fn from(id: P2PNodeId, ip: IpAddr, port: u16) -> Self {
        P2PPeer {
            id,
            ip,
            port
        }
    }

    pub fn serialize(&self) -> String {
        match &self.ip {
             IpAddr::V4(ip4) => {
                 (format!("{:x}IP4{:03}{:03}{:03}{:03}{:05}", self.id.get_id(), ip4.octets()[0], ip4.octets()[1], ip4.octets()[2], ip4.octets()[3], self.port )[..]).to_string()
             },
             IpAddr::V6(ip6) => {
                (format!("{:x}IP6{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:05}", self.id.get_id(), ip6.octets()[0], ip6.octets()[1], ip6.octets()[2], ip6.octets()[3], ip6.octets()[4], ip6.octets()[5], ip6.octets()[6], ip6.octets()[7],ip6.octets()[8], ip6.octets()[9], ip6.octets()[10], ip6.octets()[11], ip6.octets()[12], ip6.octets()[13], ip6.octets()[14], ip6.octets()[15], self.port)[..]).to_string()
             }
        }
    }

    pub fn deserialize(buf: &str) -> Option<P2PPeer> {
        if&buf.len() > &(PROTOCOL_NODE_ID_LENGTH+3) {
            let node_id = P2PNodeId::from_string(buf[..PROTOCOL_NODE_ID_LENGTH].to_string());
            let ip_type = &buf[PROTOCOL_NODE_ID_LENGTH..(PROTOCOL_NODE_ID_LENGTH+3)];
            let ip_start = PROTOCOL_NODE_ID_LENGTH+3;
            match ip_type {
                "IP4" => {
                    if&buf.len() >= &(PROTOCOL_NODE_ID_LENGTH+3+12+5) {
                         match IpAddr::from_str(&format!("{}.{}.{}.{}", &buf[ip_start..(ip_start+3)], &buf[(ip_start+3)..(ip_start+6)],
                            &buf[(ip_start+6)..(ip_start+9)],&buf[(ip_start+9)..(ip_start+12)])[..]) {
                                Ok(ip_addr) => {
                                    match buf[(ip_start+12)..(ip_start+17)].parse::<u16>() {
                                        Ok(port) => {
                                            let _node_id = P2PNodeId::from_ip_port(ip_addr, port);
                                            if _node_id.get_id() == node_id.get_id() {
                                                return Some(P2PPeer{id: node_id, ip: ip_addr, port: port})
                                            } else {
                                                return None
                                            }
                                        },
                                        Err(_) => return None
                                    }
                                },
                                Err(_) => return None
                            };
                    } else {
                        return None;
                    }
                },
                "IP6" => {
                    if&buf.len() >= &(PROTOCOL_NODE_ID_LENGTH+3+32+5) {
                         match IpAddr::from_str(&format!("{}:{}:{}:{}:{}:{}:{}:{}", &buf[ip_start..(ip_start+4)], &buf[(ip_start+4)..(ip_start+8)],
                            &buf[(ip_start+8)..(ip_start+12)],&buf[(ip_start+12)..(ip_start+16)],&buf[(ip_start+16)..(ip_start+20)],
                            &buf[(ip_start+20)..(ip_start+24)],&buf[(ip_start+24)..(ip_start+28)],&buf[(ip_start+28)..(ip_start+32)])[..]) {
                                Ok(ip_addr) => {
                                    match buf[(ip_start+32)..(ip_start+37)].parse::<u16>() {
                                        Ok(port) => {
                                            let _node_id = P2PNodeId::from_ip_port(ip_addr, port);
                                            if _node_id.get_id() == node_id.get_id() {
                                                return Some(P2PPeer{id: node_id, ip: ip_addr, port: port})
                                            } else {
                                                return None
                                            }
                                        },
                                        Err(_) => return None
                                    }
                                },
                                Err(_) => return None
                            };
                    } else {
                        return None;
                    }
                },
                _ => None
            }
        } else {
            None
        }
    }

    pub fn id(&self) -> P2PNodeId {
        self.id.clone()
    }

    pub fn ip(&self) -> IpAddr {
        self.ip
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl PartialEq for P2PPeer {
    fn eq(&self, other: &P2PPeer) -> bool {
        self.id.id == other.id().id
    }
}

impl Ord for P2PPeer {
    fn cmp(&self, other: &P2PPeer) -> Ordering {
        self.id.id.cmp(&other.id().id)
    }
}

impl PartialOrd for P2PPeer {
    fn partial_cmp(&self, other: &P2PPeer) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for P2PPeer {
}

#[derive(Debug, Clone)]
pub struct P2PNodeId {
    id: BigUint,
}

impl P2PNodeId {
    pub fn from_string(sid: String) -> P2PNodeId {
        P2PNodeId {
            id: match BigUint::from_str_radix(&sid, 16) {
                Ok(x) => {
                    x
                },
                Err(_) => {
                    panic!("Couldn't convert ID from hex to biguint")
                }
            }
        }
    }

    pub fn get_id(&self) -> &BigUint {
        &self.id
    }

    pub fn to_string(self) -> String {
        format!("{:x}", self.id)
    }

    pub fn from_ip_port(ip: IpAddr, port: u16) -> P2PNodeId {
        let ip_port = format!("{}:{}", ip, port);
        P2PNodeId::from_string(utils::to_hex_string(utils::sha256(&ip_port)))
    }

    pub fn from_ipstring(ip_port: String) -> P2PNodeId {
        P2PNodeId::from_string(utils::to_hex_string(utils::sha256(&ip_port)))
    }
}

#[cfg(test)]
mod tests {
    use common::*;
    #[test]
    pub fn req_ping_test() {
        const TEST_VALUE:&str = "CONCORDIUMP2P00100013f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP401001001001009999";
        let self_peer:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let test_msg = NetworkRequest::Ping(self_peer);
        let serialized_val = test_msg.serialize();
        assert_eq!(TEST_VALUE, serialized_val );
        let deserialized = NetworkMessage::deserialize(&serialized_val[..]);
        assert! ( match deserialized {
            NetworkMessage::NetworkRequest(NetworkRequest::Ping(_)) => true,
            _ => false
        } )
    }

    #[test]
    pub fn resp_pong_test() {
        const TEST_VALUE:&str = "CONCORDIUMP2P00110013f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP401001001001009999";
        let self_peer:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let test_msg = NetworkResponse::Pong(self_peer);
        let serialized_val = test_msg.serialize();
        assert_eq!(TEST_VALUE, serialized_val );
        let deserialized = NetworkMessage::deserialize(&serialized_val[..]);
        assert! ( match deserialized {
            NetworkMessage::NetworkResponse(NetworkResponse::Pong(_)) => true,
            _ => false
        } )
    }

    #[test]
    pub fn req_findnode_test() {
        const TEST_VALUE: &str = "CONCORDIUMP2P00100023f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP401001001001009999da8b507f3e99f5ba979c4db6d65719add14884d581e0565fb8a7fb1a7fc7a54b";
        let self_peer:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let node_id = P2PNodeId::from_ipstring("8.8.8.8:9999".to_string());
        let msg = NetworkRequest::FindNode(self_peer, node_id.clone());
        let serialized = msg.serialize();
        assert_eq!(TEST_VALUE, serialized);
        let deserialized = NetworkMessage::deserialize(&serialized[..]);
        assert! ( match deserialized {
            NetworkMessage::NetworkRequest(NetworkRequest::FindNode(_, id)) => id.get_id() == node_id.get_id(),
            _ => false
        } )
    }

    #[test]
    pub fn resp_findnode_empty_test() {
        const TEST_VALUE: &str = "CONCORDIUMP2P00110023f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP401001001001009999000";
        let self_peer:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let msg = NetworkResponse::FindNode(self_peer, vec![]);
        let serialized = msg.serialize();
        assert_eq!(TEST_VALUE, serialized);
        let deserialized = NetworkMessage::deserialize(&serialized[..]);
        assert! ( match deserialized {
            NetworkMessage::NetworkResponse(NetworkResponse::FindNode(_, peers)) => peers.len() == 0 ,
            _ => false
        } )
    }

    #[test]
    pub fn resp_findnode_v4_test() {
        const TEST_VALUE: &str = "CONCORDIUMP2P00110023f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP401001001001009999001da8b507f3e99f5ba979c4db6d65719add14884d581e0565fb8a7fb1a7fc7a54bIP400800800800809999";
        let self_peer:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let port:u16 = 9999;
        let ipaddr = IpAddr::from_str("8.8.8.8").unwrap();
        let msg = NetworkResponse::FindNode(self_peer, vec![P2PPeer::new(ipaddr,port)]);
        let serialized = msg.serialize();
        assert_eq!(TEST_VALUE, serialized);
        let deserialized = NetworkMessage::deserialize(&serialized[..]);
        assert! ( match deserialized {
            NetworkMessage::NetworkResponse(NetworkResponse::FindNode(_, peers)) => peers.len() == 1 && peers.get(0).unwrap().ip == ipaddr && peers.get(0).unwrap().port == port ,
            _ => false
        } )
    }

    #[test]
    pub fn resp_findnode_v6_test() {
        const TEST_VALUE: &str = "CONCORDIUMP2P00110023f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP401001001001009999001f255ab0bf31f86bb745ec10a125530130be80865b02a6020df5f754a1f840842IP6ff8000000000000000000000deadbeaf09999";
        let self_peer:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let port:u16 = 9999;
        let ipaddr = IpAddr::from_str("ff80::dead:beaf").unwrap();  
        let msg = NetworkResponse::FindNode(self_peer, vec![P2PPeer::new(ipaddr,port)]);
        let serialized = msg.serialize();
        assert_eq!(TEST_VALUE, serialized);
        let deserialized = NetworkMessage::deserialize(&serialized[..]);
        assert! ( match deserialized {
            NetworkMessage::NetworkResponse(NetworkResponse::FindNode(_, peers)) =>  peers.len() == 1 && peers.get(0).unwrap().ip == ipaddr && peers.get(0).unwrap().port == port,
            _ => false
        } )
    }
    
    #[test]
    pub fn resp_findnode_mixed_test() {
        const TEST_VALUE: &str = "CONCORDIUMP2P00110023f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP401001001001009999002f255ab0bf31f86bb745ec10a125530130be80865b02a6020df5f754a1f840842IP6ff8000000000000000000000deadbeaf09999da8b507f3e99f5ba979c4db6d65719add14884d581e0565fb8a7fb1a7fc7a54bIP400800800800809999";
        let self_peer:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let port:u16 = 9999;
        let ipaddr1 = IpAddr::from_str("ff80::dead:beaf").unwrap();  
        let ipaddr2 = IpAddr::from_str("8.8.8.8").unwrap();
        let msg = NetworkResponse::FindNode(self_peer, vec![P2PPeer::new(ipaddr1,port), P2PPeer::new(ipaddr2, port)]);
        let serialized = msg.serialize();
        assert_eq!(TEST_VALUE, serialized);
        let deserialized = NetworkMessage::deserialize(&serialized[..]);
        assert! ( match deserialized {
            NetworkMessage::NetworkResponse(NetworkResponse::FindNode(_, peers)) =>  peers.len() == 2,
            _ => false
        } )
    }

    #[test]
    pub fn direct_message_test() {
        const TEST_VALUE:&str = "CONCORDIUMP2P00120013f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP4010010010010099990000000012Hello world!";
        let self_peer:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let text_msg = String::from("Hello world!");
        let msg = NetworkPacket::DirectMessage(self_peer, text_msg.clone());
        let serialized = msg.serialize();
        assert_eq!(TEST_VALUE, serialized);
        let deserialized = NetworkMessage::deserialize(&serialized[..]);
        assert!( match deserialized {
            NetworkMessage::NetworkPacket(NetworkPacket::DirectMessage(_, msg)) => text_msg == msg,
            _ => false
        })
    }

    #[test]
    pub fn broadcasted_message_test() {
        const TEST_VALUE:&str = "CONCORDIUMP2P00120023f0c4f9ec9cbbef8d020d7b6d8ac600a8f8e6d0716cd2b7c6bf99c84c42ef489IP4010010010010099990000000025Hello  broadcasted world!";
        let SELF_PEER:P2PPeer = P2PPeer::new(IpAddr::from_str("10.10.10.10").unwrap(), 9999);
        let text_msg = String::from("Hello  broadcasted world!");
        let msg = NetworkPacket::BroadcastedMessage(SELF_PEER, text_msg.clone());
        let serialized = msg.serialize();
        assert_eq!(TEST_VALUE, serialized);
        let deserialized = NetworkMessage::deserialize(&serialized[..]);
        assert!( match deserialized {
            NetworkMessage::NetworkPacket(NetworkPacket::BroadcastedMessage(_, msg)) => text_msg == msg,
            _ => false
        })
    }

    #[test]
    pub fn resp_invalid_version() {
        const TEST_VALUE:&str = "CONCORDIUMP2P0021001";
        let deserialized = NetworkMessage::deserialize(TEST_VALUE);
        assert! ( match deserialized {
            NetworkMessage::InvalidMessage => true,
            _ => false
        } )
    }

    #[test]
    pub fn resp_invalid_protocol() {
        const TEST_VALUE:&str = "CONC0RD1UMP2P0021001";
        let deserialized = NetworkMessage::deserialize(TEST_VALUE);
        assert! ( match deserialized {
            NetworkMessage::InvalidMessage => true,
            _ => false
        } )
    }

    #[test]
    pub fn resp_unknown_message() {
        const TEST_VALUE:&str = "CONCORDIUMP2P0015555";
        let deserialized = NetworkMessage::deserialize(TEST_VALUE);
        assert! ( match deserialized {
            NetworkMessage::UnknownMessage => true,
            _ => false
        } )
    }
}