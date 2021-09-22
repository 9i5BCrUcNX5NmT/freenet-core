//! A in-memory connection manager and transport implementation. Used for testing pourpouses.
use std::{array::IntoIter, collections::HashMap, io::Cursor, sync::Arc, time::Duration};

use crossbeam::channel::{self, Receiver, Sender};
use once_cell::sync::OnceCell;
use parking_lot::{Mutex, RwLock};

use super::{ConnError, Transport};
use crate::{
    config::tracing::Logger,
    conn_manager::{self, ConnectionBridge, ListenerHandle, PeerKey, PeerKeyLocation},
    message::{Message, Transaction, TransactionTypeId},
    ring::Location,
};

type InboundListenerFn =
    Box<dyn Fn(PeerKeyLocation, Message) -> conn_manager::Result<()> + Send + Sync>;
type InboundListenerRegistry = RwLock<HashMap<ListenerHandle, InboundListenerFn>>;

type ResponseListenerFn =
    Box<dyn Fn(PeerKeyLocation, Message) -> conn_manager::Result<()> + Send + Sync>;
type OutboundListenerRegistry = Arc<RwLock<HashMap<Transaction, ResponseListenerFn>>>;

#[derive(Clone)]
pub(crate) struct MemoryConnManager {
    /// listeners for inbound initial messages
    inbound_listeners: Arc<HashMap<TransactionTypeId, InboundListenerRegistry>>,
    /// listeners for outbound messages replies
    outbound_listeners: OutboundListenerRegistry,
    transport: InMemoryTransport,
    // LIFO stack for pending listeners
    pend_listeners: Sender<(Transaction, ResponseListenerFn)>,
}

impl MemoryConnManager {
    pub fn new(is_open: bool, peer: PeerKey, location: Option<Location>) -> Self {
        Logger::init_logger();
        let (pend_listeners, rcv_pend_listeners) = channel::unbounded();
        let transport = InMemoryTransport::new(is_open, peer, location);
        let inbound_listeners: Arc<HashMap<TransactionTypeId, InboundListenerRegistry>> = Arc::new(
            IntoIter::new(TransactionTypeId::enumeration())
                .map(|id| (id, RwLock::new(HashMap::new())))
                .collect(),
        );
        let outbound_listeners: Arc<RwLock<HashMap<Transaction, ResponseListenerFn>>> =
            Arc::new(RwLock::new(HashMap::new()));

        let tr_cp = transport.clone();
        let inbound_cp = Arc::clone(&inbound_listeners);
        let outbound_cp = outbound_listeners.clone();
        tokio::spawn(async move {
            // evaluate the messages as they arrive
            loop {
                let msg = { tr_cp.msg_stack_queue.lock().pop() };
                if let Some(msg) = msg {
                    let msg_data: Message =
                        bincode::deserialize_from(Cursor::new(msg.data)).unwrap();
                    if let Some(tx_fn) = outbound_cp.read().get(msg_data.id()) {
                        log::debug!("Received response for transaction: {}", msg_data.id());
                        if let Some(location) = msg.origin_loc {
                            if let Err(err) = tx_fn(
                                PeerKeyLocation {
                                    peer: msg.origin,
                                    location: Some(location),
                                },
                                msg_data,
                            ) {
                                log::error!("Error processing response: {}", err);
                            }
                        } else {
                            log::error!("No location for responding peer {}", msg.target);
                        }
                    } else {
                        let listeners = &inbound_cp[&msg_data.msg_type()];
                        log::debug!("Received inbound transaction: {}", msg_data.id());
                        let reg = &*listeners.read();
                        for func in reg.values() {
                            if let Err(err) = func(
                                PeerKeyLocation {
                                    peer: msg.origin,
                                    location: None,
                                },
                                msg_data.clone(),
                            ) {
                                log::error!("Error while calling inbound msg handler: {}", err);
                            }
                        }
                    }
                    // insert any pending functions generated from within the callback
                    let mut lock = outbound_cp.write();
                    for (tx, func) in rcv_pend_listeners.try_iter() {
                        lock.insert(tx, func);
                    }
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        Self {
            inbound_listeners,
            outbound_listeners,
            transport,
            pend_listeners,
        }
    }
}

#[async_trait::async_trait]
impl ConnectionBridge for MemoryConnManager {
    async fn recv(&self) -> Result<Message, ConnError> {
        todo!()
    }

    async fn send(&self, target: &PeerKeyLocation, msg: Message) -> Result<(), ConnError> {
        todo!()
    }

    fn add_connection(&mut self, peer: PeerKeyLocation, unsolicited: bool) {
        todo!()
    }
}

// impl ConnectionBridge for MemoryConnManager {
//     type Transport = InMemoryTransport;

//     fn listen<F>(&self, tx_type: TransactionTypeId, listen_fn: F) -> ListenerHandle
//     where
//         F: Fn(PeerKeyLocation, Message) -> conn_manager::Result<()> + Send + Sync + 'static,
//     {
//         let tx_ty_listener = &self.inbound_listeners[&tx_type];
//         let handle_id = ListenerHandle::new();
//         tx_ty_listener
//             .write()
//             .insert(handle_id, Box::new(listen_fn));
//         handle_id
//     }

//     fn listen_to_replies<F>(&self, tx: Transaction, callback: F)
//     where
//         F: Fn(PeerKeyLocation, Message) -> conn_manager::Result<()> + Send + Sync + 'static,
//     {
//         // optimistically try to acquire a lock
//         if let Some(mut lock) = self.outbound_listeners.try_write() {
//             lock.insert(tx, Box::new(callback));
//         } else {
//             // it failed, this is being inserted from an other existing closure holding the lock
//             // send it to the temporal stack queue for posterior insertion
//             self.pend_listeners
//                 .send((tx, Box::new(callback)))
//                 .expect("full or disconnected");
//         }
//     }

//     fn transport(&self) -> &Self::Transport {
//         &self.transport
//     }

//     fn add_connection(&self, _peer_key: PeerKeyLocation, _unsolicited: bool) {}

//     fn send_with_callback<F>(
//         &self,
//         to: PeerKeyLocation,
//         tx: Transaction,
//         msg: Message,
//         callback: F,
//     ) -> conn_manager::Result<()>
//     where
//         F: Fn(PeerKeyLocation, Message) -> conn_manager::Result<()> + Send + Sync + 'static,
//     {
//         // store listening func
//         self.outbound_listeners
//             .write()
//             .insert(tx, Box::new(callback));

//         // send the msg
//         let serialized = bincode::serialize(&msg)?;
//         self.transport
//             .send(to.peer, to.location.unwrap(), serialized);
//         Ok(())
//     }

//     fn send(
//         &self,
//         to: PeerKeyLocation,
//         _tx: Transaction,
//         msg: Message,
//     ) -> conn_manager::Result<()> {
//         let serialized = bincode::serialize(&msg)?;
//         self.transport
//             .send(to.peer, to.location.unwrap(), serialized);
//         Ok(())
//     }

//     fn remove_listener(&self, tx: Transaction) {
//         self.outbound_listeners.write().remove(&tx);
//     }
// }

static NETWORK_WIRES: OnceCell<(Sender<MessageOnTransit>, Receiver<MessageOnTransit>)> =
    OnceCell::new();

#[derive(Clone, Debug)]
struct MessageOnTransit {
    origin: PeerKey,
    origin_loc: Option<Location>,
    target: PeerKey,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct InMemoryTransport {
    interface_peer: PeerKey,
    location: Option<Location>,
    is_open: bool,
    /// received messages per each peer awaiting processing
    msg_stack_queue: Arc<Mutex<Vec<MessageOnTransit>>>,
    /// all messages 'traversing' the network at a given time
    network: Sender<MessageOnTransit>,
}

impl InMemoryTransport {
    fn new(is_open: bool, interface_peer: PeerKey, location: Option<Location>) -> Self {
        let msg_stack_queue = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = NETWORK_WIRES.get_or_init(crossbeam::channel::unbounded);

        // store messages incoming from the network in the msg stack
        let rcv_msg_c = msg_stack_queue.clone();
        let network = tx.clone();
        let rx = rx.clone();
        tokio::spawn(async move {
            loop {
                match rx.try_recv() {
                    Ok(msg) if msg.target == interface_peer => {
                        log::debug!(
                            "Inbound message received for peer {} from {}",
                            interface_peer,
                            msg.origin
                        );
                        rcv_msg_c.lock().push(msg);
                    }
                    Err(channel::TryRecvError::Disconnected) => break,
                    Err(channel::TryRecvError::Empty) | Ok(_) => {
                        tokio::time::sleep(Duration::from_millis(1)).await
                    }
                }
            }
            log::error!("Stopped receiving messages in {}", interface_peer);
        });

        Self {
            interface_peer,
            location,
            is_open,
            msg_stack_queue,
            network,
        }
    }

    fn send(&self, peer: PeerKey, location: Location, message: Vec<u8>) {
        let send_res = self.network.try_send(MessageOnTransit {
            origin: self.interface_peer,
            origin_loc: Some(location),
            target: peer,
            data: message,
        });
        match send_res {
            Err(channel::TrySendError::Disconnected(_)) => {
                log::debug!("Network shutdown")
            }
            Err(channel::TrySendError::Full(_)) => {
                unreachable!("not unbounded capacity!")
            }
            Ok(_) => {}
        }
    }
}

impl Transport for InMemoryTransport {
    fn is_open(&self) -> bool {
        self.is_open
    }
}