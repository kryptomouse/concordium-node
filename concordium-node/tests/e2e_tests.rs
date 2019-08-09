extern crate p2p_client;

#[cfg(test)]
mod tests {
    use concordium_common::{make_atomic_callback, safe_write, UCursor};
    use failure::{bail, Fallible};
    use p2p_client::{
        common::PeerType,
        connection::network_handler::message_processor::MessageManager,
        network::{NetworkId, NetworkMessage, NetworkPacket, NetworkPacketType},
        p2p::{banned_nodes::BannedNode, p2p_node::*},
        test_utils::{
            await_handshake, connect, consume_pending_messages, get_test_config,
            make_node_and_sync, max_recv_timeout, next_available_port, setup_logger,
            wait_broadcast_message, wait_direct_message, wait_direct_message_timeout,
        },
    };

    use log::{debug, info};
    use rand::{distributions::Standard, thread_rng, Rng};
    use std::{
        collections::hash_map::DefaultHasher,
        hash::Hasher,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, RwLock,
        },
        time,
    };

    /// Counter implementation
    #[derive(Clone)]
    pub struct Counter(pub Arc<AtomicUsize>);

    impl Counter {
        /// Creates a new `Counter` starting with the given value.
        pub fn new(value: usize) -> Self { Counter(Arc::new(AtomicUsize::new(value))) }

        /// Retrieves the current counter value.
        pub fn get(&self) -> usize { self.0.load(Ordering::SeqCst) }

        /// Increase the current counter by `ticks`.
        pub fn tick(&self, ticks: usize) { self.0.fetch_add(ticks, Ordering::SeqCst); }
    }

    #[test]
    pub fn e2e_000_two_nodes() -> Fallible<()> {
        setup_logger();

        let msg = b"Hello other brother!".to_vec();
        let networks = vec![100];

        let (mut node_1, msg_waiter_1) =
            make_node_and_sync(next_available_port(), networks.clone(), PeerType::Node)?;
        let (node_2, _msg_waiter_2) =
            make_node_and_sync(next_available_port(), networks, PeerType::Node)?;
        connect(&mut node_1, &node_2)?;
        await_handshake(&msg_waiter_1)?;
        consume_pending_messages(&msg_waiter_1);

        send_direct_message(
            &node_2,
            Some(node_1.id()),
            NetworkId::from(100),
            None,
            msg.clone(),
        )?;
        let mut msg_recv = wait_direct_message(&msg_waiter_1)?;
        assert_eq!(msg.as_slice(), msg_recv.read_all_into_view()?.as_slice());

        Ok(())
    }

    #[test]
    pub fn e2e_001_two_nodes_wrong_net() -> Fallible<()> {
        setup_logger();

        let networks_1 = vec![100];
        let networks_2 = vec![200];
        let msg = b"Hello other brother!".to_vec();

        let (mut node_1, msg_waiter_1) =
            make_node_and_sync(next_available_port(), networks_1, PeerType::Node)?;
        let (node_2, _) = make_node_and_sync(next_available_port(), networks_2, PeerType::Node)?;
        connect(&mut node_1, &node_2)?;
        await_handshake(&msg_waiter_1)?;
        consume_pending_messages(&msg_waiter_1);
        // Send msg
        send_direct_message(
            &node_2,
            Some(node_1.id()),
            NetworkId::from(100),
            None,
            msg.clone(),
        )?;
        let received_msg = wait_direct_message_timeout(&msg_waiter_1, max_recv_timeout());
        assert_eq!(received_msg, Some(UCursor::from(msg)));

        Ok(())
    }

    #[test]
    pub fn e2e_002_small_mesh_net() -> Fallible<()> {
        const MESH_NODE_COUNT: usize = 15;
        setup_logger();
        let message_counter = Counter::new(0);
        let mut peers: Vec<(P2PNode, _)> = Vec::with_capacity(MESH_NODE_COUNT);

        let msg = b"Hello other mother's brother";
        // Create mesh net
        for _node_idx in 0..MESH_NODE_COUNT {
            let inner_counter = message_counter.clone();

            let (mut node, waiter) =
                make_node_and_sync(next_available_port(), vec![100], PeerType::Node)?;
            let port = node.internal_addr().port();
            node.message_processor()
                .add_notification(make_atomic_callback!(move |m: &NetworkMessage| {
                    if let NetworkMessage::NetworkPacket(pac, _, _) = m {
                        if let NetworkPacketType::BroadcastedMessage(..) = pac.packet_type {
                            inner_counter.tick(1);
                            info!(
                                "BroadcastedMessage/{}/{:?} at {} with size {} received, ticks {}",
                                pac.network_id,
                                pac.message_id,
                                port,
                                pac.message.len(),
                                inner_counter.get()
                            );
                        }
                    }
                    Ok(())
                }));

            for (tgt_node, tgt_waiter) in &peers {
                connect(&mut node, &tgt_node)?;
                await_handshake(&waiter)?;
                consume_pending_messages(&waiter);
                consume_pending_messages(&tgt_waiter);
            }

            peers.push((node, waiter));
        }

        // Send broadcast message from 0 node
        if let Some((ref node, _)) = peers.get_mut(0) {
            send_broadcast_message(node, vec![], NetworkId::from(100), None, msg.to_vec())?;
        }

        // Wait for broadcast message from 1..MESH_NODE_COUNT
        // and close and join to all nodes (included first node).
        for (node, waiter) in peers.iter_mut().skip(1) {
            let msg_recv = wait_broadcast_message(&waiter)?.read_all_into_view()?;
            assert_eq!(msg_recv.as_slice(), msg);
            assert_eq!(true, node.close_and_join().is_ok());
        }
        if let Some((ref mut node, _)) = peers.get_mut(0) {
            assert_eq!(true, node.close_and_join().is_ok());
        }

        // Check counter.
        let local_message_counter = message_counter.get();
        debug!("Check message counter: {}", local_message_counter);
        assert_eq!(MESH_NODE_COUNT - 1, local_message_counter);
        Ok(())
    }

    fn islands_mesh_test(island_size: usize, islands_count: usize) -> Fallible<()> {
        setup_logger();

        let message_counter = Counter::new(0);
        let message_count_estimated = (island_size - 1) * islands_count;

        let mut islands: Vec<Vec<(P2PNode, _)>> = Vec::with_capacity(islands_count);
        let networks = vec![100];

        let msg = b"Hello other mother's brother";
        // Create island of nodes. Each node (in each island) is connected to all
        // previous created nodes.
        for _island in 0..islands_count {
            let mut peers_islands_and_ports: Vec<(P2PNode, _)> = Vec::with_capacity(island_size);

            for _island_idx in 0..island_size {
                let inner_counter = message_counter.clone();

                let (mut node, waiter) =
                    make_node_and_sync(next_available_port(), networks.clone(), PeerType::Node)?;
                let port = node.internal_addr().port();

                node.message_processor()
                    .add_notification(make_atomic_callback!(move |m: &NetworkMessage| {
                        if let NetworkMessage::NetworkPacket(pac, _, _) = m {
                            if let NetworkPacketType::BroadcastedMessage(..) = pac.packet_type {
                                inner_counter.tick(1);
                                info!(
                                    "BroadcastedMessage/{}/{:?} at {} with size {} received, \
                                     ticks {}",
                                    pac.network_id,
                                    pac.message_id,
                                    port,
                                    pac.message.len(),
                                    inner_counter.get()
                                );
                            }
                        }
                        Ok(())
                    }));

                // Connect to previous nodes and clean any pending message in waiters
                for (tgt_node, tgt_waiter) in &peers_islands_and_ports {
                    connect(&mut node, &tgt_node)?;
                    await_handshake(&waiter)?;
                    consume_pending_messages(&waiter);
                    consume_pending_messages(&tgt_waiter);
                }
                peers_islands_and_ports.push((node, waiter));
            }
            islands.push(peers_islands_and_ports);
        }

        // Send broadcast message in each island.

        for island in &mut islands {
            if let Some((ref node_sender_ref, _)) = island.get_mut(0) {
                send_broadcast_message(
                    node_sender_ref,
                    vec![],
                    NetworkId::from(100),
                    None,
                    msg.to_vec(),
                )?;
            };
        }

        // Wait reception of that broadcast message.
        for island in islands.iter_mut() {
            for (node, waiter) in island.iter_mut().skip(1) {
                let msg_recv = wait_broadcast_message(&waiter)?.read_all_into_view()?;
                assert_eq!(msg_recv.as_slice(), msg);
                assert_eq!(true, node.close_and_join().is_ok());
            }
        }

        let local_message_counter: usize = message_counter.get();
        assert_eq!(message_count_estimated, local_message_counter);
        Ok(())
    }

    #[test]
    pub fn e2e_002_small_mesh_three_islands_net() -> Fallible<()> { islands_mesh_test(3, 3) }

    #[test]
    pub fn e2e_003_big_mesh_three_islands_net() -> Fallible<()> { islands_mesh_test(5, 3) }

    /// This test has been used in
    #[test]
    fn e2e_004_noise_ready_writeable() -> Fallible<()> {
        setup_logger();
        let msg = UCursor::from(b"Direct message between nodes".to_vec());
        let networks = vec![100];

        // 1. Create and connect nodes
        let (mut node_1, msg_waiter_1) =
            make_node_and_sync(next_available_port(), networks.clone(), PeerType::Node)?;
        let (node_2, msg_waiter_2) =
            make_node_and_sync(next_available_port(), networks, PeerType::Node)?;
        connect(&mut node_1, &node_2)?;
        await_handshake(&msg_waiter_1)?;

        // 2. Send message from n1 to n2.
        send_message_from_cursor(
            &node_1,
            Some(node_2.id()),
            vec![],
            NetworkId::from(100),
            None,
            msg.clone(),
            false,
        )?;
        let msg_1 = wait_direct_message(&msg_waiter_2)?;
        assert_eq!(msg_1, msg);

        send_message_from_cursor(
            &node_2,
            Some(node_1.id()),
            vec![],
            NetworkId::from(100),
            None,
            msg.clone(),
            false,
        )?;
        let msg_2 = wait_direct_message(&msg_waiter_1)?;
        assert_eq!(msg_2, msg);

        send_message_from_cursor(
            &node_1,
            Some(node_2.id()),
            vec![],
            NetworkId::from(102),
            None,
            msg.clone(),
            false,
        )?;
        let msg_3 = wait_direct_message(&msg_waiter_2)?;
        assert_eq!(msg_3, msg);

        Ok(())
    }

    #[test]
    pub fn e2e_004_01_close_and_join_on_not_spawned_node() -> Fallible<()> {
        setup_logger();

        let (net_tx, _) = std::sync::mpsc::sync_channel(64);
        let (rpc_tx, _) = std::sync::mpsc::sync_channel(64);
        let mut node = P2PNode::new(
            None,
            &get_test_config(next_available_port(), vec![100]),
            net_tx,
            None,
            PeerType::Node,
            None,
            rpc_tx,
        );

        assert_eq!(true, node.close_and_join().is_err());
        assert_eq!(true, node.close_and_join().is_err());
        assert_eq!(true, node.close_and_join().is_err());
        Ok(())
    }

    #[test]
    pub fn e2e_004_02_close_and_join_on_spawned_node() -> Fallible<()> {
        setup_logger();

        let (mut node_1, waiter_1) =
            make_node_and_sync(next_available_port(), vec![100], PeerType::Node)?;
        let (node_2, waiter_2) =
            make_node_and_sync(next_available_port(), vec![100], PeerType::Node)?;
        connect(&mut node_1, &node_2)?;
        await_handshake(&waiter_1)?;

        let msg = b"Hello";
        send_direct_message(
            &node_1,
            Some(node_2.id()),
            NetworkId::from(100),
            None,
            msg.to_vec(),
        )?;
        node_1.close_and_join()?;

        let node_2_msg = wait_direct_message(&waiter_2)?.read_all_into_view()?;
        assert_eq!(node_2_msg.as_slice(), msg);
        Ok(())
    }

    #[test]
    pub fn e2e_004_03_close_from_inside_spawned_node() -> Fallible<()> {
        setup_logger();

        let (mut node_1, waiter_1) =
            make_node_and_sync(next_available_port(), vec![100], PeerType::Node)?;
        let (node_2, waiter_2) =
            make_node_and_sync(next_available_port(), vec![100], PeerType::Node)?;

        let node_2_cloned = RwLock::new(node_2.clone());
        node_2
            .message_processor()
            .add_packet_action(make_atomic_callback!(move |_pac: &NetworkPacket| {
                let join_status = safe_write!(node_2_cloned)?.close_and_join();
                assert_eq!(join_status.is_err(), true);
                Ok(())
            }));
        connect(&mut node_1, &node_2)?;
        await_handshake(&waiter_1)?;

        let msg = b"Hello";
        send_direct_message(
            &node_1,
            Some(node_2.id()),
            NetworkId::from(100),
            None,
            msg.to_vec(),
        )?;

        let node_2_msg = wait_direct_message(&waiter_2)?.read_all_into_view()?;
        assert_eq!(node_2_msg.as_slice(), msg);
        Ok(())
    }

    #[test]
    pub fn e2e_005_drop_on_ban() -> Fallible<()> {
        setup_logger();

        let networks = vec![100];

        let (mut node_1, msg_waiter_1) =
            make_node_and_sync(next_available_port(), networks.clone(), PeerType::Node)?;
        let (node_2, _msg_waiter_2) =
            make_node_and_sync(next_available_port(), networks.clone(), PeerType::Node)?;
        connect(&mut node_1, &node_2)?;
        await_handshake(&msg_waiter_1)?;
        consume_pending_messages(&msg_waiter_1);

        let to_ban = BannedNode::ById(node_2.id());

        node_1.ban_node(to_ban);
        let mut reply = node_1.get_peer_stats(&vec![]);

        let t1 = time::Instant::now();
        while reply.len() == 1 {
            reply = node_1.get_peer_stats(&vec![]);
            if time::Instant::now().duration_since(t1).as_secs() > 30 {
                bail!("timeout");
            }
        }

        Ok(())
    }

    #[test]
    pub fn e2e_005_network_direct_128k() { p2p_net(128 * 1024); }

    #[test]
    pub fn e2e_005_network_direct_8m() { p2p_net(8 * 1024 * 1024); }

    #[test]
    pub fn e2e_005_network_direct_32m() { p2p_net(32 * 1024 * 1024); }

    fn p2p_net(size: usize) {
        setup_logger();

        // Create nodes and connect them.
        let (mut node_1, msg_waiter_1) =
            make_node_and_sync(next_available_port(), vec![100], PeerType::Node).unwrap();
        let (node_2, msg_waiter_2) =
            make_node_and_sync(next_available_port(), vec![100], PeerType::Node).unwrap();
        connect(&mut node_1, &node_2).unwrap();
        await_handshake(&msg_waiter_1).unwrap();

        // let mut msg = make_direct_message_into_disk().unwrap();
        let msg = thread_rng()
            .sample_iter(&Standard)
            .take(size)
            .collect::<Vec<u8>>();
        let mut uc = UCursor::from(msg);
        let net_id = NetworkId::from(100);

        // Send.
        send_message_from_cursor(
            &node_1,
            Some(node_2.id()),
            vec![],
            net_id,
            None,
            uc.clone(),
            false,
        )
        .unwrap();
        let mut msg_recv = wait_direct_message(&msg_waiter_2).unwrap();
        assert_eq!(uc.len(), msg_recv.len());

        // Get content hash.
        let content_hash_list = [
            uc.read_all_into_view().unwrap(),
            msg_recv.read_all_into_view().unwrap(),
        ]
        .into_iter()
        .map(|view| {
            let mut hasher = DefaultHasher::new();
            hasher.write(view.as_slice());
            hasher.finish()
        })
        .collect::<Vec<u64>>();

        assert_eq!(content_hash_list[0], content_hash_list[1]);
    }
}
